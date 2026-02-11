use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::rngs::OsRng;

#[derive(Clone)]
pub struct PasswordPolicy {
    pub memory_kb: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl PasswordPolicy {
    pub fn argon2(&self) -> Argon2<'static> {
        use argon2::{Algorithm, Params, Version};
        let params = Params::new(self.memory_kb, self.iterations, self.parallelism, None)
            .expect("invalid argon2 params");
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
    }
}

pub fn hash_password(policy: &PasswordPolicy, password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = policy.argon2();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string();
    Ok(hash)
}

pub fn verify_password(policy: &PasswordPolicy, password: &str, stored_hash: &str) -> Result<bool, String> {
    let parsed = PasswordHash::new(stored_hash).map_err(|e| e.to_string())?;
    let argon2 = policy.argon2();
    Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
}
