use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};

pub fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32]; // 256-bit
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_refresh_token(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}
