use base64::prelude::*;
use getrandom::getrandom;
use sha2::{Digest, Sha256};

/// Generate a new secure token (32 bytes, base64url encoded)
pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom(&mut bytes).map_err(|e| format!("Failed to generate random bytes: {}", e))?;
    Ok(BASE64_URL_SAFE_NO_PAD.encode(bytes))
}

/// Hash a token using SHA-256
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result)
}

/// Constant-time comparison of token hashes
pub fn verify_token_hash(provided_hash: &str, stored_hash: &str) -> bool {
    use subtle::ConstantTimeEq;

    if provided_hash.len() != stored_hash.len() {
        return false;
    }

    provided_hash
        .as_bytes()
        .ct_eq(stored_hash.as_bytes())
        .into()
}

// Add hex dependency
mod hex {
    use std::fmt::Write;

    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut s, "{:02x}", byte).unwrap();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        let token = generate_token().unwrap();
        assert!(!token.is_empty());
        assert!(token.len() >= 32);
    }

    #[test]
    fn test_hash_token() {
        let token = "test_token";
        let hash = hash_token(token);
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars
    }

    #[test]
    fn test_verify_token_hash() {
        let token = "test_token";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);
        assert!(verify_token_hash(&hash1, &hash2));

        let wrong_hash = hash_token("wrong_token");
        assert!(!verify_token_hash(&hash1, &wrong_hash));
    }
}
