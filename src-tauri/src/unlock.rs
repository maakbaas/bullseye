//! The unlock password for a forced stop, and the passwords of password locks (`new_hash`).
//!
//! Only a salted PBKDF2-HMAC-SHA256 hash of it is in the executable, so it cannot be read out of
//! the binary as text. What remains: a short or common password can still be found by trying
//! candidates against the hash, so a long one is what makes this hold.
//!
//! To change it: `$env:UNLOCK_PASSWORD='...'; cargo test print_unlock_hash -- --ignored --nocapture`
//! and paste the printed `SALT` and `HASH` here.

use pbkdf2::pbkdf2_hmac_array;
use sha2::Sha256;

/// OWASP's 2023 figure for PBKDF2-HMAC-SHA256.
const ROUNDS: u32 = 600_000;

const SALT: [u8; 16] = [104, 116, 83, 68, 28, 116, 108, 94, 229, 34, 162, 94, 158, 187, 1, 138];
const HASH: [u8; 32] = [
    140, 119, 92, 178, 177, 104, 164, 78, 225, 131, 53, 187, 133, 110, 35, 193, 74, 144, 33, 62, 106, 245, 255, 33, 43, 40, 44, 209, 76,
    158, 238, 199,
];

fn hash(password: &str, salt: &[u8]) -> [u8; 32] {
    pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), salt, ROUNDS)
}

pub fn check(password: &str) -> bool {
    same(&hash(password, &SALT), &HASH)
}

/// Compared without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (a, b)| diff | (a ^ b)) == 0
}

/// The password of a password lock (`lock::Lock::Password`), as it is kept: a new random salt and
/// the hash, `salt$hash` in hex. The password itself is never kept.
pub fn new_hash(password: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| format!("cannot make a salt: {e}"))?;
    Ok(format!("{}${}", hex(&salt), hex(&hash(password, &salt))))
}

/// Whether `password` is the one `kept` (from `new_hash`) was made from.
pub fn check_hash(password: &str, kept: &str) -> bool {
    let Some((salt, expected)) = kept.split_once('$').and_then(|(s, h)| Some((unhex(s)?, unhex(h)?))) else { return false };
    same(&hash(password, &salt), &expected)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn only_the_password_unlocks() {
        assert!(check("test1234"));
        for wrong in ["", "test123", "test12345", "TEST1234", " test1234"] {
            assert!(!check(wrong), "{wrong:?}");
        }
    }

    #[test]
    fn a_lock_password_is_kept_only_as_a_salted_hash() {
        let kept = new_hash("correct horse").unwrap();
        assert!(!kept.contains("correct horse"));
        assert!(check_hash("correct horse", &kept));
        assert!(!check_hash("correct hors", &kept) && !check_hash("", &kept) && !check_hash("correct horse", "nonsense"));
        // A new salt each time: the same password does not give the same hash.
        assert_ne!(new_hash("correct horse").unwrap(), kept);
    }

    /// Prints a new `SALT` and `HASH` for `$env:UNLOCK_PASSWORD`.
    #[test]
    #[ignore]
    fn print_unlock_hash() {
        let password = std::env::var("UNLOCK_PASSWORD").expect("set UNLOCK_PASSWORD");
        let mut seed = Sha256::new();
        seed.update(format!("{:?} {}", std::time::SystemTime::now(), std::process::id()));
        seed.update(std::fs::read(std::env::current_exe().unwrap()).unwrap_or_default());
        let salt: [u8; 16] = seed.finalize()[..16].try_into().unwrap();
        println!("const SALT: [u8; 16] = {salt:?};\nconst HASH: [u8; 32] = {:?};", hash(&password, &salt));
    }
}
