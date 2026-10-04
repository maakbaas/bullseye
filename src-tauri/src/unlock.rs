//! The unlock password for a forced stop.
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
    140, 119, 92, 178, 177, 104, 164, 78, 225, 131, 53, 187, 133, 110, 35, 193, 74, 144, 33, 62, 106, 245, 255, 33, 43, 40,
    44, 209, 76, 158, 238, 199,
];

fn hash(password: &str, salt: &[u8]) -> [u8; 32] {
    pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), salt, ROUNDS)
}

pub fn check(password: &str) -> bool {
    // Compared without stopping at the first difference.
    hash(password, &SALT).iter().zip(HASH).fold(0, |diff, (a, b)| diff | (a ^ b)) == 0
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
