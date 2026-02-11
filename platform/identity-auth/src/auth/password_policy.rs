use std::collections::HashSet;

#[derive(Clone)]
pub struct PasswordRules {
    pub min_len: usize,
    pub require_upper: bool,
    pub require_lower: bool,
    pub require_digit: bool,
    pub require_symbol: bool,
    pub denylist: HashSet<String>,
}

impl Default for PasswordRules {
    fn default() -> Self {
        // Keep v1 simple but real:
        // - strong minimum length
        // - basic character class checks
        // - denylist hook
        let denylist = HashSet::from([
            "password".to_string(),
            "password123".to_string(),
            "1234567890".to_string(),
            "qwerty".to_string(),
            "letmein".to_string(),
        ]);

        Self {
            min_len: 12,
            require_upper: true,
            require_lower: true,
            require_digit: true,
            require_symbol: false, // optional for v1; set true if you want
            denylist,
        }
    }
}

#[derive(Debug)]
pub enum PasswordPolicyError {
    TooShort { min_len: usize },
    Denylisted,
    MissingUpper,
    MissingLower,
    MissingDigit,
    MissingSymbol,
}

impl std::fmt::Display for PasswordPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasswordPolicyError::TooShort { min_len } => write!(f, "password too short (min {min_len})"),
            PasswordPolicyError::Denylisted => write!(f, "password is too common"),
            PasswordPolicyError::MissingUpper => write!(f, "password must include an uppercase letter"),
            PasswordPolicyError::MissingLower => write!(f, "password must include a lowercase letter"),
            PasswordPolicyError::MissingDigit => write!(f, "password must include a digit"),
            PasswordPolicyError::MissingSymbol => write!(f, "password must include a symbol"),
        }
    }
}

pub fn validate_password(rules: &PasswordRules, password: &str) -> Result<(), PasswordPolicyError> {
    let p = password.trim();

    if p.len() < rules.min_len {
        return Err(PasswordPolicyError::TooShort { min_len: rules.min_len });
    }

    let lowered = p.to_lowercase();
    if rules.denylist.contains(&lowered) {
        return Err(PasswordPolicyError::Denylisted);
    }

    let has_upper = p.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = p.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = p.chars().any(|c| c.is_ascii_digit());
    let has_symbol = p.chars().any(|c| !c.is_ascii_alphanumeric());

    if rules.require_upper && !has_upper {
        return Err(PasswordPolicyError::MissingUpper);
    }
    if rules.require_lower && !has_lower {
        return Err(PasswordPolicyError::MissingLower);
    }
    if rules.require_digit && !has_digit {
        return Err(PasswordPolicyError::MissingDigit);
    }
    if rules.require_symbol && !has_symbol {
        return Err(PasswordPolicyError::MissingSymbol);
    }

    Ok(())
}
