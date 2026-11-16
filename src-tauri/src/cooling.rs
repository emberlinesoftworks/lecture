// keeps transcription from heating up the mac during a long class.
// the macbook air has no fan, so if we push it the whole thing gets warm and slows down anyway

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum Heat {
    Normal,
    // macos calls this "serious": time to back off
    Warm,
    // macos calls this "critical": stop until it cools down
    Hot,
}

// asks macos how warm it's running
#[cfg(target_os = "macos")]
pub fn current_heat() -> Heat {
    use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};

    match NSProcessInfo::processInfo().thermalState() {
        NSProcessInfoThermalState::Serious => Heat::Warm,
        NSProcessInfoThermalState::Critical => Heat::Hot,
        _ => Heat::Normal,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn current_heat() -> Heat {
    Heat::Normal
}

// QoS classes from <sys/qos.h>
#[cfg(target_os = "macos")]
const QOS_CLASS_USER_INITIATED: u32 = 0x19;
#[cfg(target_os = "macos")]
const QOS_CLASS_UTILITY: u32 = 0x11;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

// during class: "utility" is what macos uses for background work, so this mostly
// runs on the efficiency cores and never fights with whatever you're doing.
// whisper's own threads are started from this one, so they get it too
pub fn run_in_background() {
    #[cfg(target_os = "macos")]
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }
}

// after you press stop you're waiting on it, so let it use the fast cores
pub fn hurry_up() {
    #[cfg(target_os = "macos")]
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0);
    }
}

// waits, but stops early if the recording ends so you're never left waiting on a break
pub fn rest(length: Duration, finished: &AtomicBool) {
    let step = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    while waited < length && !finished.load(Ordering::SeqCst) {
        thread::sleep(step);
        waited += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn rest_ends_right_away_once_the_recording_stops() {
        let finished = AtomicBool::new(true);
        let started = Instant::now();
        rest(Duration::from_secs(20), &finished);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn rest_waits_while_still_recording() {
        let finished = AtomicBool::new(false);
        let started = Instant::now();
        rest(Duration::from_millis(500), &finished);
        assert!(started.elapsed() >= Duration::from_millis(500));
    }
}
