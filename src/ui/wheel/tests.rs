//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn wheel_calc() {
    assert_eq!(calc_step(1, 200), 1);
    assert_eq!(calc_step(3, 100), 2);
    assert_eq!(calc_step(5, 100), 5);
    assert_eq!(calc_step(3, 50), 5);
}

#[test]
fn wheel_state_progression() {
    let mut ws = WheelState::new();
    let base = Instant::now();
    let s1 = ws.push(base, 120);
    assert_eq!(s1, 1);
    let s2 = ws.push(base + Duration::from_millis(50), 120);
    assert_eq!(s2, 5);
    let s3 = ws.push(base + Duration::from_millis(100), 120);
    assert_eq!(s3, 5);
    let mut ws2 = WheelState::new();
    let b = Instant::now();
    assert_eq!(ws2.push(b, 120), 1);
    assert_eq!(ws2.push(b + Duration::from_millis(90), 120), 1);
    assert_eq!(ws2.push(b + Duration::from_millis(180), 120), 2);
    assert_eq!(ws2.push(b + Duration::from_millis(270), 120), 2);
    let mut ws3 = WheelState::new();
    let s = ws3.push(b, 240);
    // 240 is 2 ticks but still single event — effective_count=2, still 1
    assert_eq!(s, 1);
}

#[test]
fn wheel_i32_min() {
    let mut ws = WheelState::new();
    let b = Instant::now();
    // Should not panic
    let s = ws.push(b, i32::MIN);
    assert!(s == 1 || s == 2 || s == 5);
    assert_eq!(WheelState::total_step(i32::MIN, 1), i32::MIN / 120);
}

#[test]
fn wheel_large_delta() {
    let mut ws = WheelState::new();
    let b = Instant::now();
    let s = ws.push(b, 480);
    // 480 = 4 ticks, effective_count = 1 + 3 =4 -> step 2
    assert_eq!(s, 2);
}

/// EarTrumpet-style hover: `Leave` clears the burst history so a stale burst
/// cannot jump the volume on the next hover.
#[test]
fn hover_leave_resets_wheel_acceleration() {
    let mut wheel = WheelState::new();
    let base = Instant::now();
    assert_eq!(wheel.push(base, 120), 1);
    assert_eq!(wheel.push(base + Duration::from_millis(50), 120), 5);
    wheel.clear();
    let later = base + Duration::from_millis(300);
    assert_eq!(wheel.push(later, 120), 1);
}

#[test]
fn total_step_sign_and_scaling() {
    // Single tick keeps sign with per-tick step.
    assert_eq!(WheelState::total_step(120, 1), 1);
    assert_eq!(WheelState::total_step(-120, 1), -1);
    // Sub-tick delta still yields one signed step.
    assert_eq!(WheelState::total_step(60, 2), 2);
    assert_eq!(WheelState::total_step(-60, 2), -2);
    // Multi-tick scales linearly with sign.
    assert_eq!(WheelState::total_step(240, 2), 4);
    assert_eq!(WheelState::total_step(-240, 5), -10);
}
