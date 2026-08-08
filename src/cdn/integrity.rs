use base64::Engine;
use sha2::{Digest, Sha256};

pub fn calculate_integrity(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(hash)
    )
}
