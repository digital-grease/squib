//! Plan phase 8 (ADR-066): the primers' text, its figures, and the angle converter.

use squib_ballistics::RangePoint;
use squib_ballistics::angle::{AngleUnit, INCH_M, YARD_M};
use squib_mobile::{SquibError, convert_angle, primer_by_id, primers};

fn text(id: &str) -> String {
    let p = primer_by_id(id.into()).unwrap_or_else(|| panic!("primer {id}"));
    let mut all = p.paragraphs.join("\n");
    all.push('\n');
    all.push_str(&p.in_squib);
    all
}

#[test]
fn every_primer_is_short_plain_and_linked() {
    let all = primers();
    assert_eq!(all.len(), 11, "the confirmed scope: 6 range card, 2 calculator, 3 timer");
    let ids: Vec<_> = all.iter().map(|p| p.id.clone()).collect();
    for p in &all {
        assert_eq!(ids.iter().filter(|i| **i == p.id).count(), 1, "{} is unique", p.id);
        assert!(!p.title.is_empty() && !p.summary.is_empty() && !p.paragraphs.is_empty() && !p.in_squib.is_empty());
        let words = p.paragraphs.iter().chain([&p.in_squib]).map(|t| t.split_whitespace().count()).sum::<usize>();
        assert!(words <= 200, "{} is short ({words} words)", p.id);
        for t in p.paragraphs.iter().chain([&p.title, &p.summary, &p.in_squib]) {
            assert!(!t.contains('\u{2014}'), "{}: no em dashes", p.id);
            assert!(!t.contains("NaN") && !t.contains("inf"), "{}: every figure computed", p.id);
        }
        for r in &p.related {
            assert!(ids.contains(r) && r != &p.id, "{} links to {r}", p.id);
        }
        assert_eq!(primer_by_id(p.id.clone()).as_ref(), Some(p));
    }
    assert_eq!(all.iter().filter(|p| p.converter).count(), 1, "one converter, in MOA and mil");
    assert!(primer_by_id("nope".into()).is_none());
}

/// The figures in the text match the published definitions, computed here independently
/// of the angle module: 1 MOA = pi/10800 rad, 1 mil = 1/1000 rad, 1 IPHY = 1 in at 100 yd.
#[test]
fn figures_match_published_definitions() {
    let moa = std::f64::consts::PI / 10800.0;
    let yd100_in = 3600.0;
    let units = text("angle-units");
    assert!(units.contains(&format!("covers {:.3} in at 100 yd", (moa).tan() * yd100_in)), "{units}");
    assert!(units.contains("1.047 in at 100 yd") && units.contains("2.91 cm at 100 m"));
    assert!(units.contains("covers 10.0 cm at 100 m, 100 cm at 1000 m, or 3.6 in at 100 yd"));
    assert!(units.contains("One mil is 3.438 MOA; one MOA is 0.291 mil"));
    assert!(units.contains("10.5 in at 1000 yd"));

    let iphy = text("shooters-moa");
    assert!(iphy.contains("about 4.7 percent"), "{iphy}");
    assert!(iphy.contains("300 in on an IPHY scope and 314 in with true MOA, a difference of 14 in"), "{iphy}");
    assert!(iphy.contains("multiply Squib's MOA by 1.047"));

    let clicks = text("clicks");
    assert!(clicks.contains("2.4 mil is 24 clicks of 0.1 mil, and 6.5 MOA is 26 clicks of 1/4 MOA"), "{clicks}");

    let wind = text("drop-windage");
    assert!(wind.contains("has 0.50 of that") && wind.contains("10 o'clock 0.87"), "{wind}");

    // ICAO standard atmosphere: 340.294 m/s at 15 C.
    let mach = text("transonic");
    assert!(mach.contains("340 m/s (1116 fps) at 15 °C"), "{mach}");
    assert!(mach.contains("below Mach 1.2 (408 m/s or 1340 fps at 15 °C)"), "{mach}");
    assert!(mach.contains("below Mach 1.2 as \"less reliable\""));

    let hf = text("hit-factor");
    assert!(
        hf.contains(
            "10 points in 2.10 s is a hit factor of 4.762. 10 points in 1.90 s is 5.263, but 8 points in 1.90 s is 4.211"
        ),
        "{hf}"
    );

    let splits = text("timer-terms");
    assert!(splits.contains("splits of 0.45 and 0.45 s"), "{splits}");
}

/// The range card and the converter give the same angle for the same miss.
#[test]
fn converter_agrees_with_the_range_card() {
    // A range-card point as the solver builds it: the correction subtends the miss.
    let range_m = 600.0 * YARD_M;
    let miss_m = -2.25;
    let el = -(miss_m / range_m).atan();
    let point = RangePoint {
        range_m,
        drop_m: miss_m,
        windage_m: 0.0,
        spin_drift_m: 0.0,
        elevation_mil: AngleUnit::Mil.from_rad(el),
        elevation_moa: AngleUnit::Moa.from_rad(el),
        windage_mil: 0.0,
        windage_moa: 0.0,
        velocity_mps: 500.0,
        mach: 1.5,
        time_s: 0.8,
        energy_j: 2000.0,
        transonic: false,
    };
    let c = convert_angle(point.elevation_mil, "mil".into(), range_m, true).unwrap();
    assert!((c.size_m - 2.25).abs() < 1e-9, "the converter puts the miss back");
    assert!((c.moa - point.elevation_moa).abs() < 1e-9);
    let c = convert_angle(point.elevation_moa, "moa".into(), range_m, false).unwrap();
    assert!((c.mil - point.elevation_mil).abs() < 1e-9);
}

#[test]
fn converter_examples_and_refusals() {
    // The worked example in the plan: a 600 yd hold of 4.1 mil.
    let c = convert_angle(4.1, "mil".into(), 600.0 * YARD_M, true).unwrap();
    assert_eq!(c.clicks_tenth_mil, 41);
    assert_eq!(c.clicks_quarter_moa, 56);
    assert!((c.size_m / INCH_M - 88.56).abs() < 0.01, "{}", c.size_m / INCH_M);
    assert_eq!(
        c.sentence,
        "4.1 mil at 600 yd covers 88.6 in. That is 4.10 mil, 14.09 MOA or 14.76 IPHY: 41 clicks of 0.1 mil, or 56 clicks of 1/4 MOA."
    );
    let c = convert_angle(1.0, "mil".into(), 100.0, false).unwrap();
    assert!(c.sentence.starts_with("1 mil at 100 m covers 10.0 cm."), "{}", c.sentence);
    let c = convert_angle(30.0, "iphy".into(), 1000.0 * YARD_M, true).unwrap();
    assert!(c.sentence.starts_with("30 IPHY at 1000 yd covers 300.0 in."), "{}", c.sentence);
    assert!(convert_angle(0.0, "moa".into(), 100.0, false).unwrap().sentence.starts_with("0 MOA at 100 m covers 0.0 cm."));

    let rejected = |r: Result<_, SquibError>| matches!(r, Err(SquibError::Rejected(_)));
    assert!(rejected(convert_angle(-1.0, "mil".into(), 100.0, false)));
    assert!(rejected(convert_angle(f64::NAN, "mil".into(), 100.0, false)));
    assert!(rejected(convert_angle(601.0, "mil".into(), 100.0, false)));
    assert!(rejected(convert_angle(1.0, "mil".into(), 0.0, false)));
    assert!(rejected(convert_angle(1.0, "mil".into(), 3001.0, false)));
    assert!(matches!(convert_angle(1.0, "degrees".into(), 100.0, false), Err(SquibError::Invalid(_))));
}

/// Prints every primer; run with `--ignored --nocapture` to read them.
#[test]
#[ignore]
fn dump() {
    for p in primers() {
        println!("## {} ({})\n{}\n", p.title, p.id, p.summary);
        for t in &p.paragraphs {
            println!("{t}\n");
        }
        println!("In Squib: {}\nRelated: {:?}\nSources: {:?}\n", p.in_squib, p.related, p.sources);
    }
}
