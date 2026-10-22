//! In-app primers (plan phase 8, owner ruling D23, ADR-066): short offline explanations of
//! the terms the app shows. Every figure in the text is computed here from the same
//! functions the solver uses (`squib_ballistics::angle`, `solver::speed_of_sound`,
//! `TRANSONIC_MACH`), so the text cannot drift from the calculator.

use squib_ballistics::angle::{AngleUnit, INCH_M, YARD_M, ratio, subtension_m};
use squib_ballistics::{TRANSONIC_MACH, solver::speed_of_sound};

use crate::SquibError;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrimerView {
    pub id: String,
    pub title: String,
    /// One line for the list of primers.
    pub summary: String,
    pub paragraphs: Vec<String>,
    /// What Squib does with this, shown last.
    pub in_squib: String,
    /// Shows the angle converter.
    pub converter: bool,
    /// Ids of related primers.
    pub related: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AngleConversionView {
    pub mil: f64,
    pub moa: f64,
    pub iphy: f64,
    /// What the angle covers at the distance, metres.
    pub size_m: f64,
    pub clicks_tenth_mil: i64,
    pub clicks_quarter_moa: i64,
    /// The whole result as one sentence, in the app's units.
    pub sentence: String,
}

/// Farthest distance the converter accepts, as the solver's range limit.
const MAX_DISTANCE_M: f64 = squib_ballistics::MAX_RANGE_M;
/// Largest angle the converter accepts, in mil (a little over 34 degrees).
const MAX_MIL: f64 = 600.0;
/// Reference temperature for the speed-of-sound figures: 15 C, the ISA sea-level value.
const REFERENCE_K: f64 = 288.15;

fn f(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}")
}

/// Inches covered by `value` of `unit` at `yards`.
fn inches_at(unit: AngleUnit, value: f64, yards: f64) -> f64 {
    subtension_m(unit.to_rad(value), yards * YARD_M) / INCH_M
}

fn inches_at_100yd(unit: AngleUnit, value: f64) -> f64 {
    inches_at(unit, value, 100.0)
}

/// A number with up to two decimals and no trailing zeros: "4.1", "30", "0".
fn short(v: f64) -> String {
    let s = f(v, 2);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.into() }
}

/// Centimetres covered by one `unit` at 100 m.
fn cm_at_100m(unit: AngleUnit) -> f64 {
    subtension_m(unit.to_rad(1.0), 100.0) * 100.0
}

fn clicks(value: f64, unit: AngleUnit, click: f64, click_unit: AngleUnit) -> i64 {
    (click_unit.from_rad(unit.to_rad(value)) / click).round() as i64
}

#[allow(clippy::too_many_arguments)] // Each primer reads as one call, in field order.
fn primer(
    id: &str,
    title: &str,
    summary: &str,
    paragraphs: Vec<String>,
    in_squib: String,
    converter: bool,
    related: &[&str],
    sources: &[&str],
) -> PrimerView {
    PrimerView {
        id: id.into(),
        title: title.into(),
        summary: summary.into(),
        paragraphs,
        in_squib,
        converter,
        related: related.iter().map(|s| (*s).into()).collect(),
        sources: sources.iter().map(|s| (*s).into()).collect(),
    }
}

const NIST_ANGLE: &str = "NIST Special Publication 811 (2008), Guide for the Use of the International System of Units: the radian, the degree and the minute of arc";
const LITZ: &str = "Bryan Litz, Applied Ballistics for Long-Range Shooting, 3rd edition (2015): ballistic coefficients and the G1 and G7 standards";
const MCCOY: &str = "Robert L. McCoy, Modern Exterior Ballistics (1999): standard drag functions and transonic flight";
const ICAO_ATMOSPHERE: &str =
    "ICAO Doc 7488/3, Manual of the ICAO Standard Atmosphere: speed of sound and the 15 C sea-level reference";

fn all() -> Vec<PrimerView> {
    use AngleUnit::{Iphy, Mil, Moa};
    let a_mil = speed_of_sound(REFERENCE_K);
    let fps = |mps: f64| mps / 0.3048;
    let sixty = 60f64.to_radians().sin();
    let thirty = 30f64.to_radians().sin();
    let iphy_30 = inches_at(Iphy, 30.0, 1000.0);
    let moa_30 = inches_at(Moa, 30.0, 1000.0);
    let hf = |points: f64, s: f64| f(points / s, 3);
    vec![
        primer(
            "angle-units",
            "MOA and mil (MRAD)",
            "The two angle units scopes and range cards use.",
            vec![
                "Scopes correct for drop and wind by angle, not by distance, because the same angle covers more ground the farther away the target is. Two units are common.".into(),
                format!(
                    "A minute of angle (MOA) is 1/60 of a degree. It covers {} in at 100 yd, {} in at 1000 yd, or {} cm at 100 m.",
                    f(inches_at_100yd(Moa, 1.0), 3),
                    f(inches_at(Moa, 1.0, 1000.0), 1),
                    f(cm_at_100m(Moa), 2),
                ),
                format!(
                    "A milliradian (mil or MRAD) is 1/1000 of a radian. It covers {} cm at 100 m, {} cm at 1000 m, or {} in at 100 yd.",
                    f(cm_at_100m(Mil), 1),
                    f(subtension_m(Mil.to_rad(1.0), 1000.0) * 100.0, 0),
                    f(inches_at_100yd(Mil, 1.0), 1),
                ),
                format!(
                    "One mil is {} MOA; one MOA is {} mil. Neither is better, but your scope's turrets and reticle use one of them. Use the same one on your range card, so you never convert in your head.",
                    f(ratio(Mil, Moa), 3),
                    f(ratio(Moa, Mil), 3),
                ),
            ],
            "Squib computes each correction as an angle and shows it in mil or MOA, whichever you pick with \"Corrections in mil\" on the Ballistics screen. The converter below uses the same calculation.".into(),
            true,
            &["shooters-moa", "clicks", "focal-plane"],
            &[NIST_ANGLE],
        ),
        primer(
            "shooters-moa",
            "Shooter's MOA (IPHY)",
            "Why some MOA scopes are a little off from true MOA.",
            vec![
                format!(
                    "Some scopes adjust in inches per hundred yards (IPHY), sometimes sold as \"shooter's MOA\": exactly 1 in at 100 yd, instead of the {} in of a true MOA.",
                    f(inches_at_100yd(Moa, 1.0), 3),
                ),
                format!(
                    "The difference is about {} percent: small for one click, but it adds up. Dialing 30 units at 1000 yd moves the shot {} in on an IPHY scope and {} in with true MOA, a difference of {} in, enough to miss a small target.",
                    f((ratio(Moa, Iphy) - 1.0) * 100.0, 1),
                    f(iphy_30, 0),
                    f(moa_30, 0),
                    f(moa_30 - iphy_30, 0),
                ),
                "The scope's manual says which one it uses. A turret marked \"1/4 in at 100 yd\" per click is IPHY.".into(),
            ],
            format!(
                "Squib's MOA is true MOA. If your scope uses IPHY, multiply Squib's MOA by {} to get IPHY, or use the converter in \"MOA and mil\".",
                f(ratio(Moa, Iphy), 3),
            ),
            false,
            &["angle-units", "clicks"],
            &[NIST_ANGLE],
        ),
        primer(
            "clicks",
            "Clicks, holding and dialing",
            "Turning a correction into turret clicks or a reticle hold.",
            vec![
                "A turret moves the reticle in clicks of a fixed angle, marked on the turret: often 0.1 mil or 1/4 MOA, sometimes 1/8 MOA.".into(),
                format!(
                    "To dial a correction, divide it by the click value: 2.4 mil is {} clicks of 0.1 mil, and 6.5 MOA is {} clicks of 1/4 MOA. Then aim with the centre of the reticle.",
                    clicks(2.4, Mil, 0.1, Mil),
                    clicks(6.5, Moa, 0.25, Moa),
                ),
                "To hold instead, leave the turrets alone and aim with a reticle mark that far from the centre: for 2.4 mil up, put the mark 2.4 mil below the centre on the target. Holding is quicker between targets; dialing keeps the centre for careful shots. Holds are only right if the reticle's marks use the same unit as the card and, on many scopes, only at one magnification.".into(),
            ],
            "Range-card corrections are shown to 0.1 mil or 0.1 MOA; on a 1/4 MOA turret, round to the nearest click. The converter in \"MOA and mil\" counts clicks for 0.1 mil and 1/4 MOA turrets.".into(),
            false,
            &["focal-plane", "angle-units", "drop-windage"],
            &[],
        ),
        primer(
            "focal-plane",
            "First and second focal plane",
            "When reticle holds are true, and when they are not.",
            vec![
                "A variable-power scope has its reticle in one of two places. In a first focal plane (FFP) scope the reticle grows and shrinks with the image, so a mark that covers 1 mil covers 1 mil at every magnification.".into(),
                "In a second focal plane (SFP) scope the reticle stays the same size while the image changes, so its marks cover their stated angle at one magnification only, often the highest; the manual says which. At other settings the angle changes in proportion: on a scope whose reticle is true at 12x, at 6x each mark covers twice its stated value.".into(),
                "Turret clicks are not affected; only holds with the reticle are.".into(),
            ],
            "Squib's holds assume the reticle's marks are true. On an SFP scope, hold at the magnification where the reticle is true, or dial instead.".into(),
            false,
            &["clicks"],
            &[],
        ),
        primer(
            "zero",
            "Zero and sight height",
            "Why the bullet crosses your line of sight, and where.",
            vec![
                "The sight sits above the bore, so the bullet starts below the line of sight, rises across it, and falls back across it farther out. Your zero distance is where it comes back down to the line of sight: the bullet hits exactly where you aim.".into(),
                "Up close, the bullet is below your aim by nearly the full sight height. With a scope 1.5 in above the bore, a shot at a few yards lands about 1.5 in low. That is why the sight height matters for short shots and for working out the zero.".into(),
                "Beyond the zero, the bullet falls farther below the line of sight with every yard, and that is what the range card's corrections are for. A different zero distance changes every correction on the card.".into(),
            ],
            "Squib finds the angle that puts the bullet on the line of sight at your load's zero distance on a level shot, from the sight height you enter. Spin drift and Earth's rotation are never part of the zero.".into(),
            false,
            &["drop-windage"],
            &[],
        ),
        primer(
            "drop-windage",
            "Drop, windage and wind direction",
            "What the two corrections fix, and how much of the wind counts.",
            vec![
                "Drop is how far the bullet falls below the line of sight; the elevation correction (up) brings it back. Windage is how far the wind pushes it sideways; the wind correction (left or right) brings that back. Wind from the right pushes the bullet left, so the correction is to the right.".into(),
                format!(
                    "Only the part of the wind that blows across the shot matters much. Wind from 3 or 9 o'clock, straight from the right or left, has its full effect. From 1, 5, 7 or 11 o'clock it has {} of that, and from 2, 4, 8 or 10 o'clock {}. A headwind or tailwind, from 12 or 6, has almost no sideways effect.",
                    f(thirty, 2),
                    f(sixty, 2),
                ),
            ],
            "On the Ballistics screen you enter the wind's speed and the clock position it blows from (12 from the target, 3 from your right). Squib takes it as the same all the way to the target, which real wind rarely is.".into(),
            false,
            &["clicks"],
            &[],
        ),
        primer(
            "bc",
            "Ballistic coefficient: G1 and G7",
            "What the BC means, and why G1 and G7 numbers cannot be mixed.",
            vec![
                "A ballistic coefficient (BC) says how well a bullet keeps its speed compared with a standard reference bullet: the higher the BC, the less it slows. A BC only means something together with its reference, called its drag model.".into(),
                "G1 is an old flat-based reference shape; most hunting-bullet BCs are published as G1. G7 is a long boat-tail shape, much closer to modern match and long-range bullets, so a G7 BC stays nearly the same across speeds where a G1 BC changes. The same bullet's G1 number is roughly twice its G7 number.".into(),
                "Never enter a G7 number as G1 or the other way round. A G7 BC read as G1 makes the bullet seem to slow far faster than it does, and the card shows far too much drop.".into(),
            ],
            "Each load keeps its BC with its drag model, as the maker publishes it (lb/in²), and Squib scales the matching standard drag table (BRL/McCoy G1 or G7). Use the maker's BC for your exact bullet.".into(),
            false,
            &["transonic"],
            &[LITZ, MCCOY],
        ),
        primer(
            "transonic",
            "Transonic flight (Mach 1.2)",
            "Why distances where the bullet nears the speed of sound are less reliable.",
            vec![
                format!(
                    "Mach 1 is the speed of sound: {} m/s ({} fps) at 15 °C, a little faster in warm air and slower in cold. As a bullet slows toward Mach 1, the shock waves around it change, and its drag changes sharply.",
                    f(a_mil, 0),
                    f(fps(a_mil), 0),
                ),
                format!(
                    "In this transonic range, roughly below Mach {} ({} m/s or {} fps at 15 °C), standard drag tables fit real bullets less well, and some bullets wobble or lose stability. Predictions there are less certain.",
                    f(TRANSONIC_MACH, 1),
                    f(TRANSONIC_MACH * a_mil, 0),
                    f(fps(TRANSONIC_MACH * a_mil), 0),
                ),
            ],
            format!(
                "Squib's model was checked mainly in supersonic flight, so the range card marks every distance where the bullet is below Mach {} as \"less reliable\". Confirm those distances on paper.",
                f(TRANSONIC_MACH, 1),
            ),
            false,
            &["bc"],
            &[MCCOY, ICAO_ATMOSPHERE],
        ),
        primer(
            "timer-terms",
            "Par, first shot, splits and total",
            "The times a shot timer reports.",
            vec![
                "Par time is a time limit you set. The timer gives the start signal, then a second cue at the par; the goal is to finish before it.".into(),
                "First shot is the time from the start signal to your first shot: your draw or ready-up time. A split is the time between one shot and the next. Total is the time from the start signal to your last shot.".into(),
                format!(
                    "Example: shots at 1.20, 1.65 and 2.10 s give a first shot of 1.20 s, splits of {} and {} s, and a total of 2.10 s.",
                    f(1.65 - 1.20, 2),
                    f(2.10 - 1.65, 2),
                ),
            ],
            "In par-only mode Squib plays the start and par cues and never listens. With live shot timing it listens for the shots and works out these times. A result you enter from another timer is marked Manual.".into(),
            false,
            &["hit-factor", "phone-timing"],
            &[],
        ),
        primer(
            "hit-factor",
            "Hit factor",
            "Points divided by time: accuracy and speed in one number.",
            vec![
                "Hit factor is points divided by time in seconds, the scoring used in USPSA and IPSC matches. It rewards accuracy and speed together: going faster only helps if you keep your points.".into(),
                format!(
                    "Example: 10 points in 2.10 s is a hit factor of {}. 10 points in 1.90 s is {}, but 8 points in 1.90 s is {}, worse than the slower, cleaner run.",
                    hf(10.0, 2.10),
                    hf(10.0, 1.90),
                    hf(8.0, 1.90),
                ),
            ],
            "Squib's practice scoring works out the hit factor from the points you enter and the run's time, and never goes below zero. It is generic practice scoring, not an official match score.".into(),
            false,
            &["timer-terms"],
            &[],
        ),
        primer(
            "phone-timing",
            "Why phone timing is experimental",
            "What the timing notes on a result mean.",
            vec![
                "A shot timer is built to hear shots; a phone is not. Its microphone, audio processing and clock differ from model to model, and Squib has not yet been tested on enough phones at real ranges to say how accurate it is.".into(),
                "Times are shown to 0.01 s, but that is how they are displayed, not how accurate they are. Each result also says how the start was timed (the start cue heard by the microphone is best; the time the cue was asked to play is weaker) and how well the audio clock was tied to the phone's clock.".into(),
                "Use live timing to compare your own runs on the same phone, and check anything important against a dedicated timer.".into(),
            ],
            "Shot timing is labelled Experimental until it has been tested on real phones at the range. Shots Squib is unsure of are marked for review instead of guessed, and your corrections are kept as new revisions.".into(),
            false,
            &["timer-terms"],
            &[],
        ),
    ]
}

/// Every primer, in reading order.
#[uniffi::export]
pub fn primers() -> Vec<PrimerView> {
    all()
}

/// One primer by id.
#[uniffi::export]
pub fn primer_by_id(id: String) -> Option<PrimerView> {
    all().into_iter().find(|p| p.id == id)
}

/// Converts an angle in `mil`, `moa` or `iphy` at a distance (m): the other units, what
/// it covers there, and turret clicks. `us` picks yards and inches for the sentence.
#[uniffi::export]
pub fn convert_angle(value: f64, unit: String, distance_m: f64, us: bool) -> Result<AngleConversionView, SquibError> {
    let u = AngleUnit::parse(&unit).ok_or_else(|| SquibError::Invalid(format!("unknown angle unit {unit}")))?;
    if !value.is_finite() || value < 0.0 {
        return Err(SquibError::Rejected("Enter an angle of 0 or more.".into()));
    }
    let rad = u.to_rad(value);
    if AngleUnit::Mil.from_rad(rad) > MAX_MIL {
        return Err(SquibError::Rejected(format!("Enter an angle up to {} mil.", f(MAX_MIL, 0))));
    }
    if !distance_m.is_finite() || distance_m <= 0.0 || distance_m > MAX_DISTANCE_M {
        let max = if us { format!("{} yd", f(MAX_DISTANCE_M / YARD_M, 0)) } else { format!("{} m", f(MAX_DISTANCE_M, 0)) };
        return Err(SquibError::Rejected(format!("Enter a distance above 0 and up to {max}.")));
    }
    let size_m = subtension_m(rad, distance_m);
    let (mil, moa, iphy) = (AngleUnit::Mil.from_rad(rad), AngleUnit::Moa.from_rad(rad), AngleUnit::Iphy.from_rad(rad));
    let clicks_tenth_mil = clicks(value, u, 0.1, AngleUnit::Mil);
    let clicks_quarter_moa = clicks(value, u, 0.25, AngleUnit::Moa);
    let (distance, size) = if us {
        (format!("{} yd", f(distance_m / YARD_M, 0)), format!("{} in", f(size_m / INCH_M, 1)))
    } else {
        (format!("{} m", f(distance_m, 0)), format!("{} cm", f(size_m * 100.0, 1)))
    };
    let sentence = format!(
        "{} {} at {distance} covers {size}. That is {} mil, {} MOA or {} IPHY: {clicks_tenth_mil} clicks of 0.1 mil, or {clicks_quarter_moa} clicks of 1/4 MOA.",
        short(value),
        u.symbol(),
        f(mil, 2),
        f(moa, 2),
        f(iphy, 2),
    );
    Ok(AngleConversionView { mil, moa, iphy, size_m, clicks_tenth_mil, clicks_quarter_moa, sentence })
}
