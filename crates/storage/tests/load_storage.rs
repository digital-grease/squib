//! Schema 12: versioned ballistics loads and immutable range cards, and upgrade from
//! schema 11 (plan 6.3, ADR-062).

use std::path::PathBuf;

use squib_ballistics::{Atmosphere, DragModel, Earth, Load, Shot, Spin, Wind, solve};
use squib_environment::privacy::LocationRetention;
use squib_environment::*;
use squib_storage::*;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-loadstore-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lv(id: &str, version: u32, mv: f64) -> LoadVersion {
    LoadVersion {
        load_id: id.into(),
        version,
        name: "140 ELD-M".into(),
        equipment_id: None,
        load: Load {
            model: DragModel::G7,
            bc_lb_in2: 0.326,
            mass_kg: 0.009_07,
            muzzle_velocity_mps: mv,
            sight_height_m: 0.04,
            zero_range_m: 100.0,
        },
        bullet_diameter_m: None,
        bullet_length_m: None,
        twist_m: Some(-0.203),
        notes: String::new(),
        created_utc_ms: version as i64,
    }
}

fn card(id: &str, v: &LoadVersion, snapshot: Option<String>, session: Option<String>) -> BallisticCard {
    let shot = Shot {
        load: v.load.clone(),
        air: Atmosphere::from_station(90_000.0, 20.0, 0.3),
        zero_air: None,
        wind: Some(Wind { speed_mps: 2.0, from_deg: 270.0 }),
        look_angle_deg: 5.0,
        spin: None,
        coriolis: None,
    };
    let ranges = vec![200.0, 400.0];
    let result = solve(&shot, &ranges).unwrap();
    BallisticCard {
        id: id.into(),
        load_id: v.load_id.clone(),
        load_version: v.version,
        environment_snapshot_id: snapshot,
        session_id: session,
        title: "Plateau".into(),
        inputs: CardInputs { shot, ranges_m: ranges },
        result,
        created_utc_ms: 100,
    }
}

fn snapshot(repo: &Repository, id: &str) -> String {
    let c = vec![local::barometer(835.0, 1000).unwrap()];
    let s = resolve(id, &c, &[], None, vec![], &ResolverPolicy::default(), 1000);
    repo.insert_snapshot(&s, LocationRetention::Precise).unwrap().id
}

#[test]
fn load_versions_are_sequential_validated_and_immutable() {
    let dir = tmpdir("versions");
    let mut repo = Repository::open(&dir.join("j.db")).unwrap();
    repo.save_load_version(&lv("a", 1, 820.0)).unwrap();
    assert!(repo.save_load_version(&lv("a", 3, 821.0)).is_err(), "no gaps");
    assert!(repo.save_load_version(&lv("a", 1, 821.0)).is_err(), "no rewriting");
    repo.save_load_version(&lv("a", 2, 823.5)).unwrap();
    let latest = repo.list_loads().unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!((latest[0].version, latest[0].load.muzzle_velocity_mps), (2, 823.5));
    assert_eq!(repo.load_history("a").unwrap().iter().map(|v| v.version).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(repo.load_load_version("a", 1).unwrap().unwrap(), lv("a", 1, 820.0), "every field round-trips");
    let upd = repo.connection().execute("UPDATE load_version SET bc_lb_in2 = 0.5 WHERE load_id = 'a'", []);
    assert!(upd.is_err(), "versions are immutable");

    for bad in [
        LoadVersion { name: " ".into(), ..lv("b", 1, 800.0) },
        LoadVersion { name: "x".repeat(81), ..lv("b", 1, 800.0) },
        LoadVersion { load: Load { bc_lb_in2: 0.0, ..lv("b", 1, 800.0).load }, ..lv("b", 1, 800.0) },
        LoadVersion { load: Load { muzzle_velocity_mps: f64::NAN, ..lv("b", 1, 800.0).load }, ..lv("b", 1, 800.0) },
        LoadVersion { twist_m: Some(0.05), ..lv("b", 1, 800.0) },
        LoadVersion { bullet_diameter_m: Some(0.5), ..lv("b", 1, 800.0) },
        LoadVersion { notes: "n".repeat(1001), ..lv("b", 1, 800.0) },
        LoadVersion { equipment_id: Some("no-such-rifle".into()), ..lv("b", 1, 800.0) },
    ] {
        assert!(repo.save_load_version(&bad).is_err(), "{bad:?}");
    }
    assert!(repo.load_history("b").unwrap().is_empty(), "nothing half-saved");

    repo.save_equipment_version(&EquipmentVersion {
        equipment_id: "rifle".into(),
        version: 1,
        name: "Bolt gun".into(),
        category: "rifle".into(),
        notes: String::new(),
        created_utc_ms: 1,
    })
    .unwrap();
    repo.save_load_version(&LoadVersion { equipment_id: Some("rifle".into()), ..lv("b", 1, 800.0) }).unwrap();
    repo.archive_load("a").unwrap();
    assert_eq!(repo.list_loads().unwrap().iter().map(|v| v.load_id.as_str()).collect::<Vec<_>>(), vec!["b"]);
    assert!(repo.archive_load("zzz").is_err());
}

#[test]
fn cards_record_exact_inputs_and_cannot_be_changed() {
    let dir = tmpdir("cards");
    let mut repo = Repository::open(&dir.join("j.db")).unwrap();
    repo.save_load_version(&lv("a", 1, 820.0)).unwrap();
    repo.save_load_version(&lv("a", 2, 823.5)).unwrap();
    let snap = snapshot(&repo, "env-1");
    let c = card("c1", &lv("a", 2, 823.5), Some(snap.clone()), None);
    repo.save_ballistic_card(&c).unwrap();
    assert_eq!(repo.ballistic_card("c1").unwrap().unwrap(), c, "inputs and result exactly as computed");
    assert!(c.result.method.starts_with("squib-pointmass-v1"));

    // A card must use the load version it names, with its exact values.
    let wrong = BallisticCard { id: "c2".into(), load_version: 1, ..c.clone() };
    assert!(repo.save_ballistic_card(&wrong).is_err(), "inputs differ from version 1");
    let missing = BallisticCard { id: "c3".into(), load_version: 9, ..c.clone() };
    assert!(repo.save_ballistic_card(&missing).is_err());
    let mut no_method = BallisticCard { id: "c4".into(), ..c.clone() };
    no_method.result.method = " ".into();
    assert!(repo.save_ballistic_card(&no_method).is_err());
    assert!(
        repo.save_ballistic_card(&BallisticCard { id: "c5".into(), environment_snapshot_id: Some("nope".into()), ..c.clone() })
            .is_err()
    );

    let conn = repo.connection();
    assert!(conn.execute("UPDATE ballistic_card SET title = 'x' WHERE id = 'c1'", []).is_err(), "immutable");
    assert!(conn.execute("DELETE FROM ballistic_card WHERE id = 'c1'", []).is_err(), "deleted only on the guarded path");

    // Deleting a card removes the snapshot it pinned, once nothing else uses it.
    repo.save_ballistic_card(&BallisticCard { id: "c6".into(), created_utc_ms: 200, ..c.clone() }).unwrap();
    assert_eq!(repo.list_ballistic_cards(10).unwrap().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["c6", "c1"]);
    repo.delete_ballistic_card("c1").unwrap();
    let snaps = |r: &mut Repository| -> i64 {
        r.connection().query_row("SELECT count(*) FROM environment_snapshot", [], |x| x.get(0)).unwrap()
    };
    assert_eq!(snaps(&mut repo), 1, "still used by c6");
    repo.delete_ballistic_card("c6").unwrap();
    assert_eq!(snaps(&mut repo), 0);
    assert!(repo.delete_ballistic_card("c6").is_err());
}

#[test]
fn cards_with_optional_terms_use_the_load_versions_bullet() {
    let dir = tmpdir("optional");
    let mut repo = Repository::open(&dir.join("j.db")).unwrap();
    let no_bullet = lv("a", 1, 820.0);
    let with_bullet =
        LoadVersion { version: 2, bullet_diameter_m: Some(0.006_71), bullet_length_m: Some(0.034_8), ..lv("a", 2, 820.0) };
    repo.save_load_version(&no_bullet).unwrap();
    repo.save_load_version(&with_bullet).unwrap();
    let spin = Spin { twist_m: -0.203, diameter_m: 0.006_71, length_m: 0.034_8 };
    let with_terms = |v: &LoadVersion, id: &str, spin: Spin| {
        let mut c = card(id, v, None, None);
        c.inputs.shot.spin = Some(spin);
        c.inputs.shot.coriolis = Some(Earth { latitude_deg: 38.0, azimuth_deg: 270.0 });
        c.result = solve(&c.inputs.shot, &c.inputs.ranges_m).unwrap();
        c
    };
    let good = with_terms(&with_bullet, "c1", spin);
    assert!(good.result.stability.is_some() && good.result.method.contains("Coriolis"));
    repo.save_ballistic_card(&good).unwrap();
    assert_eq!(repo.ballistic_card("c1").unwrap().unwrap(), good, "terms, stability and spin drift round-trip");

    assert!(repo.save_ballistic_card(&with_terms(&no_bullet, "c2", spin)).is_err(), "version 1 has no bullet size");
    let other = Spin { length_m: 0.03, ..spin };
    assert!(repo.save_ballistic_card(&with_terms(&with_bullet, "c3", other)).is_err(), "not the version's bullet");
}

#[test]
fn cards_go_with_their_visit_and_with_all_history() {
    let dir = tmpdir("delete");
    let mut repo = Repository::open(&dir.join("j.db")).unwrap();
    repo.ensure_default_shooter(1).unwrap();
    repo.save_load_version(&lv("a", 1, 820.0)).unwrap();
    let s1 = repo.active_session("default", "sess-1", 1, 0).unwrap();
    let snap = snapshot(&repo, "env-s1");
    repo.save_ballistic_card(&card("in-visit", &lv("a", 1, 820.0), Some(snap), Some(s1.clone()))).unwrap();
    repo.save_ballistic_card(&card("loose", &lv("a", 1, 820.0), None, None)).unwrap();
    let rep = repo.delete_session(&s1).unwrap();
    assert_eq!(rep.cards, 1);
    assert_eq!(repo.list_ballistic_cards(10).unwrap().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["loose"]);
    let snaps: i64 = repo.connection().query_row("SELECT count(*) FROM environment_snapshot", [], |x| x.get(0)).unwrap();
    assert_eq!(snaps, 0, "the visit's card snapshot went too");

    let rep = repo.delete_all_history().unwrap();
    assert_eq!(rep.cards, 1);
    assert!(repo.list_loads().unwrap().is_empty() && repo.load_history("a").unwrap().is_empty());
}

#[test]
fn upgrade_from_schema_11() {
    let dir = tmpdir("upgrade12");
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        for (_, sql) in &MIGRATIONS[..11] {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 11).unwrap();
    }
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), 12);
    assert!(dir.join("journal.pre-v12.bak").exists());
    repo.save_load_version(&lv("a", 1, 820.0)).unwrap();
    assert_eq!(repo.list_loads().unwrap().len(), 1);
}
