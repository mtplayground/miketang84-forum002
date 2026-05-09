use argon2::{
    password_hash::{rand_core::OsRng, Error as PasswordHashError, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("password hash format is invalid: {0}")]
    InvalidHash(String),
}

#[allow(dead_code)]
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);

    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| PasswordError::Hash(err.to_string()))
}

#[allow(dead_code)]
pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, PasswordError> {
    let parsed_hash =
        PasswordHash::new(password_hash).map_err(|err| PasswordError::InvalidHash(err.to_string()))?;

    match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(PasswordHashError::Password) => Ok(false),
        Err(err) => Err(PasswordError::Hash(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_password, verify_password, PasswordError};

    #[test]
    fn password_round_trip_succeeds() {
        let password = "correct horse battery staple";
        let hash = hash_password(password).expect("hashing should succeed");

        assert!(verify_password(password, &hash).expect("verification should succeed"));
        assert!(!verify_password("wrong password", &hash).expect("mismatch should not error"));
    }

    #[test]
    fn invalid_hash_is_reported() {
        let err = verify_password("password", "not-a-valid-argon2-hash")
            .expect_err("invalid hash should return an error");

        assert!(matches!(err, PasswordError::InvalidHash(_)));
    }
}
