//! Plan phase 6.3 engine flows: ballistics loads saved as versions, the active load,
//! validation messages, equipment link, backup round trip, and deletion.

use std::sync::Arc;

use squib_mobile::ballistics_api::LoadInput;
use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;

fn engine(name: &str) -> (Arc<SquibEngine>, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("squib-ballistics-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    (SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap(), d)
}

fn input(name: &str, mv: f64) -> LoadInput {
    LoadInput {
        name: name.into(),
        equipment_id: None,
        drag_model: "G7".into(),
        bc_lb_in2: 0.243,
        bullet_mass_kg: 0.011_34,
        muzzle_velocity_mps: mv,
        sight_height_m: 0.038,
        zero_range_m: 91.44,
        bullet_diameter_m: Some(0.007_82),
        bullet_length_m: None,
        twist_m: Some(0.254),
        notes: String::new(),
    }
}

#[test]
fn loads_are_versioned_with_an_active_choice() {
    let (e, _) = engine("versions");
    let a = e.save_load(None, input("175 SMK", 790.0), NOW).unwrap();
    assert_eq!(a.version, 1);
    let a2 = e.save_load(Some(a.id.clone()), input("175 SMK", 795.5), NOW + 1).unwrap();
    assert_eq!((a2.id.as_str(), a2.version), (a.id.as_str(), 2));
    let b = e.save_load(None, input("140 ELD-M", 820.0), NOW).unwrap();
    let names: Vec<String> = e.list_loads().unwrap().into_iter().map(|v| v.input.name).collect();
    assert_eq!(names, vec!["140 ELD-M", "175 SMK"], "by name, current versions only");
    let hist = e.load_history(a.id.clone()).unwrap();
    assert_eq!(hist.iter().map(|v| v.input.muzzle_velocity_mps).collect::<Vec<_>>(), vec![790.0, 795.5]);

    e.set_active_load(Some(b.id.clone())).unwrap();
    assert!(e.list_loads().unwrap().iter().any(|v| v.id == b.id && v.active));
    assert!(e.set_active_load(Some("nope".into())).is_err());
    e.archive_load(b.id.clone()).unwrap();
    assert!(e.list_loads().unwrap().iter().all(|v| v.id != b.id && !v.active), "archiving clears the active choice");
    assert!(e.save_load(Some("nope".into()), input("x", 800.0), NOW).is_err());
}

#[test]
fn invalid_loads_are_refused_with_a_reason() {
    let (e, _) = engine("invalid");
    let cases: Vec<(LoadInput, &str)> = vec![
        (LoadInput { drag_model: "G9".into(), ..input("x", 800.0) }, "G1 or G7"),
        (LoadInput { bc_lb_in2: 0.0, ..input("x", 800.0) }, "ballistic coefficient"),
        (LoadInput { muzzle_velocity_mps: 5000.0, ..input("x", 800.0) }, "muzzle velocity"),
        (LoadInput { zero_range_m: 2.0, ..input("x", 800.0) }, "zero range"),
        (LoadInput { name: "".into(), ..input("x", 800.0) }, "name"),
        (LoadInput { twist_m: Some(2.0), ..input("x", 800.0) }, "twist"),
    ];
    for (bad, why) in cases {
        let err = e.save_load(None, bad, NOW).unwrap_err().to_string();
        assert!(err.contains(why), "{err} should mention {why}");
    }
    assert!(e.list_loads().unwrap().is_empty());
    // "g7" is accepted in any case.
    assert_eq!(
        e.save_load(None, LoadInput { drag_model: "g1".into(), ..input("x", 800.0) }, NOW).unwrap().input.drag_model,
        "G1"
    );
}

#[test]
fn equipment_link_backup_and_delete_all() {
    let (e, d) = engine("backup");
    let rifle = e.add_equipment("Bolt gun".into(), "rifle".into(), String::new(), NOW).unwrap();
    let a = e.save_load(None, LoadInput { equipment_id: Some(rifle.id.clone()), ..input("175 SMK", 790.0) }, NOW).unwrap();
    assert_eq!(a.equipment_name.as_deref(), Some("Bolt gun"));
    e.save_load(Some(a.id.clone()), input("175 SMK", 795.5), NOW + 1).unwrap();

    let zip = d.join("b.zip").to_string_lossy().to_string();
    e.export_backup(zip.clone(), d.to_string_lossy().into(), false, false, "t".into(), NOW, None).unwrap();
    let (fresh, fd) = engine("restore");
    fresh.import_backup(zip, fd.to_string_lossy().into(), None, None).unwrap();
    let hist = fresh.load_history(a.id.clone()).unwrap();
    assert_eq!(hist.iter().map(|v| (v.version, v.input.muzzle_velocity_mps)).collect::<Vec<_>>(), vec![(1, 790.0), (2, 795.5)]);
    assert_eq!(hist[0].input.equipment_id.as_deref(), Some(rifle.id.as_str()));

    e.delete_all_history().unwrap();
    assert!(e.list_loads().unwrap().is_empty());
}
