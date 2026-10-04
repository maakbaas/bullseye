//! The time a timed block is measured against: internet time (SNTP), not the Windows clock, so
//! moving the system clock forward does not end a block early.
//!
//! Once an NTP server has answered, the time is that answer plus how long the process has run
//! since, measured with `Instant` (the performance counter, which the Windows clock setting does
//! not touch). It is re-checked every few minutes, against drift and sleep.
//!
//! Before any server has answered (offline, or the first seconds after logon), the time is the
//! last moment the running block was known to be at (`at_least`, saved with the block), plus how
//! long this process has run. That only moves forward while the app is running, so unplugging the
//! network does not let the clock be moved either. The cost is the honest one: time spent with
//! the PC off counts only once the internet time is back.
//!
//! With neither, there is no block to protect, and the Windows clock is used.

use std::net::UdpSocket;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Asked in turn until one answers. Several operators, so one being down or blocked is not enough.
const SERVERS: [&str; 4] = ["time.cloudflare.com:123", "time.google.com:123", "pool.ntp.org:123", "time.windows.com:123"];

/// How long to wait for one server's answer.
const TIMEOUT: Duration = Duration::from_secs(2);

/// How often to ask again once a server has answered, and while none has.
const RESYNC: Duration = Duration::from_secs(10 * 60);
const RETRY: Duration = Duration::from_secs(10);

/// Seconds between the NTP epoch (1900) and the Unix epoch (1970).
const NTP_TO_UNIX: u64 = 2_208_988_800;

/// No answer before this (2024-01-01) is believed: it is a broken or lying server.
const EARLIEST: u64 = 1_704_067_200;

/// A known time and the moment it was known.
#[derive(Clone, Copy)]
struct Anchor {
    /// Since the Unix epoch.
    unix: Duration,
    at: Instant,
    /// From an NTP server, rather than the floor saved with a block.
    synced: bool,
}

impl Anchor {
    fn now(&self) -> Duration {
        self.unix + self.at.elapsed()
    }
}

/// The app's clock; a struct so tests can have their own.
#[derive(Default)]
pub struct Clock {
    anchor: Mutex<Option<Anchor>>,
}

static CLOCK: Clock = Clock { anchor: Mutex::new(None) };

impl Clock {
    pub fn now(&self) -> SystemTime {
        match *self.anchor.lock().unwrap() {
            Some(anchor) => UNIX_EPOCH + anchor.now(),
            None => SystemTime::now(),
        }
    }

    pub fn synced(&self) -> bool {
        self.anchor.lock().unwrap().is_some_and(|a| a.synced)
    }

    /// Internet time, as just read from a server.
    fn set(&self, unix: Duration, at: Instant) {
        *self.anchor.lock().unwrap() = Some(Anchor { unix, at, synced: true });
    }

    /// Until internet time is in, the time is at least `unix_secs`: a time the running block
    /// was seen at. Ignored once it is in.
    pub fn at_least(&self, unix_secs: u64) {
        let mut anchor = self.anchor.lock().unwrap();
        let floor = Duration::from_secs(unix_secs);
        match *anchor {
            Some(a) if a.synced || a.now() >= floor => {}
            _ => *anchor = Some(Anchor { unix: floor, at: Instant::now(), synced: false }),
        }
    }
}

/// The app's clock, which `spawn_sync` keeps on internet time.
pub fn global() -> &'static Clock {
    &CLOCK
}

/// Puts the app's clock on internet time once, now; false if no server answered.
pub fn sync() -> bool {
    query_any().map(|(unix, at)| CLOCK.set(unix, at)).is_some()
}

/// Keeps the clock on internet time, in the background.
pub fn spawn_sync() {
    thread::spawn(|| loop {
        thread::sleep(if sync() { RESYNC } else { RETRY });
    });
}

/// The first answer from `SERVERS`, with the moment it applies to.
fn query_any() -> Option<(Duration, Instant)> {
    SERVERS.iter().find_map(|server| query(server).inspect_err(|e| eprintln!("clock: {server}: {e}")).ok())
}

/// One SNTP request (RFC 4330): the server's transmit time, plus half the round trip.
fn query(server: &str) -> Result<(Duration, Instant), String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    socket.connect(server).map_err(|e| e.to_string())?;
    let mut packet = [0u8; 48];
    // Leap indicator 0, version 4, mode 3 (client).
    packet[0] = 0b00_100_011;
    let sent = Instant::now();
    socket.send(&packet).map_err(|e| e.to_string())?;
    let mut reply = [0u8; 48];
    let len = socket.recv(&mut reply).map_err(|e| e.to_string())?;
    let received = Instant::now();
    let unix = parse(&reply[..len])?;
    Ok((unix + (received - sent) / 2, received))
}

/// The transmit time of a server reply, since the Unix epoch.
fn parse(reply: &[u8]) -> Result<Duration, String> {
    if reply.len() < 48 {
        return Err(format!("short reply ({} bytes)", reply.len()));
    }
    let mode = reply[0] & 0b111;
    let stratum = reply[1];
    // Mode 4 is a server reply; stratum 0 is a "kiss of death" (go away), not a time.
    if mode != 4 || stratum == 0 || stratum > 15 {
        return Err(format!("not a usable reply (mode {mode}, stratum {stratum})"));
    }
    let seconds = u64::from(u32::from_be_bytes(reply[40..44].try_into().unwrap()));
    let fraction = u64::from(u32::from_be_bytes(reply[44..48].try_into().unwrap()));
    let unix = seconds.checked_sub(NTP_TO_UNIX).filter(|&s| s >= EARLIEST).ok_or("implausible time")?;
    Ok(Duration::from_secs(unix) + Duration::from_nanos(fraction * 1_000_000_000 >> 32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(mode: u8, stratum: u8, ntp_seconds: u32, fraction: u32) -> [u8; 48] {
        let mut r = [0u8; 48];
        r[0] = 0b00_100_000 | mode;
        r[1] = stratum;
        r[40..44].copy_from_slice(&ntp_seconds.to_be_bytes());
        r[44..48].copy_from_slice(&fraction.to_be_bytes());
        r
    }

    #[test]
    fn server_replies_are_read() {
        let t = (1_800_000_000 + NTP_TO_UNIX) as u32;
        assert_eq!(parse(&reply(4, 2, t, 1 << 31)), Ok(Duration::from_millis(1_800_000_000_500)));
        // Not a server reply, a kiss of death, a time from before 2024, too short.
        assert!(parse(&reply(3, 2, t, 0)).is_err());
        assert!(parse(&reply(4, 0, t, 0)).is_err());
        assert!(parse(&reply(4, 2, 1000, 0)).is_err());
        assert!(parse(&reply(4, 2, t, 0)[..47]).is_err());
    }

    #[test]
    fn offline_the_clock_runs_from_the_saved_floor() {
        let clock = Clock::default();
        // Far ahead of the Windows clock, as though it had been set back.
        let floor = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 86_400;
        clock.at_least(floor);
        let now = clock.now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert!((floor..floor + 2).contains(&now));
        // An older floor does not move it back.
        clock.at_least(floor - 3600);
        assert!(clock.now().duration_since(UNIX_EPOCH).unwrap().as_secs() >= floor);
        assert!(!clock.synced());
    }

    #[test]
    fn internet_time_wins_over_the_floor() {
        let clock = Clock::default();
        let real = Duration::from_secs(1_800_000_000);
        clock.set(real, Instant::now());
        clock.at_least(1_900_000_000);
        assert!(clock.synced());
        assert_eq!(clock.now().duration_since(UNIX_EPOCH).unwrap().as_secs(), 1_800_000_000);
    }

    /// Talks to the real servers: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_server_answers_with_about_the_real_time() {
        let (unix, _) = query_any().expect("no NTP server answered");
        let local = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let off = unix.as_secs_f64() - local.as_secs_f64();
        eprintln!("internet time is {off:+.3}s from the Windows clock");
        assert!(off.abs() < 300.0);
    }
}
