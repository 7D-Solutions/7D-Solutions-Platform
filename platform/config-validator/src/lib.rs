//! Shared helpers for validating module configuration from environment variables.
//!
//! This crate exposes a builder-style `ConfigValidator` that tracks every
//! missing or malformed variable and reports ALL problems at once via
//! `ConfigValidationError`.
use std::env;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

/// Collects and reports configuration problems for one module.
#[derive(Debug)]
pub struct ConfigValidator {
    module: String,
    errors: Vec<ConfigError>,
}

impl ConfigValidator {
    /// Begin validating configuration for `module_name`.
    pub fn new(module_name: impl Into<String>) -> Self {
        Self {
            module: module_name.into(),
            errors: Vec::new(),
        }
    }

    /// Require the environment variable `key` and return its value.
    ///
    /// Returns `None` if the variable is missing or empty.  In either case,
    /// `finish` will report the problem.
    pub fn require(&mut self, key: &'static str) -> Option<String> {
        match load_env(key) {
            EnvValue::Present(value) => Some(value),
            EnvValue::Empty => {
                self.push_error(key, "set but empty");
                None
            }
            EnvValue::Missing => {
                self.push_error(
                    key,
                    "required by the module but absent from the environment",
                );
                None
            }
        }
    }

    /// Read `key` if present, but do not fail if it is missing.
    pub fn optional(&mut self, key: &'static str) -> OptionalValue {
        let value = match load_env(key) {
            EnvValue::Present(val) => Some(val),
            _ => None,
        };
        OptionalValue { value }
    }

    /// Require `key` and parse it as `T`.
    ///
    /// Even when parsing fails we return `ParsedValue::empty()` so the caller
    /// can provide a default, while `finish` will still report the parse
    /// error.
    pub fn require_parse<T>(&mut self, key: &'static str) -> ParsedValue<T>
    where
        T: FromStr,
        <T as FromStr>::Err: Display,
    {
        match load_env(key) {
            EnvValue::Present(value) => match value.trim().parse::<T>() {
                Ok(parsed) => ParsedValue {
                    value: Some(parsed),
                },
                Err(err) => {
                    self.push_error(
                        key,
                        format!(
                            "must be a valid {} (got '{}'): {}",
                            type_name::<T>(),
                            value,
                            err
                        ),
                    );
                    ParsedValue { value: None }
                }
            },
            EnvValue::Empty => {
                self.push_error(key, "set but empty");
                ParsedValue { value: None }
            }
            EnvValue::Missing => {
                self.push_error(
                    key,
                    "required by the module but absent from the environment",
                );
                ParsedValue { value: None }
            }
        }
    }

    /// Read `key` and parse it as `T` if present, but do not fail if absent.
    ///
    /// Reports a parse error (via `finish`) only when the variable IS set but
    /// cannot be parsed.  If the variable is missing or empty, returns
    /// `ParsedValue { value: None }` silently.
    pub fn optional_parse<T>(&mut self, key: &'static str) -> ParsedValue<T>
    where
        T: FromStr,
        <T as FromStr>::Err: Display,
    {
        match load_env(key) {
            EnvValue::Present(value) => match value.trim().parse::<T>() {
                Ok(parsed) => ParsedValue {
                    value: Some(parsed),
                },
                Err(err) => {
                    self.push_error(
                        key,
                        format!(
                            "must be a valid {} (got '{}'): {}",
                            type_name::<T>(),
                            value,
                            err
                        ),
                    );
                    ParsedValue { value: None }
                }
            },
            EnvValue::Empty | EnvValue::Missing => ParsedValue { value: None },
        }
    }

    /// Require `key` whenever `condition` returns `true`, otherwise behave the
    /// same as `optional`.
    pub fn require_when<F>(
        &mut self,
        key: &'static str,
        condition: F,
        reason: &'static str,
    ) -> Option<String>
    where
        F: FnOnce() -> bool,
    {
        let env_value = load_env(key);
        if !condition() {
            return env_value.into_option();
        }

        match env_value {
            EnvValue::Present(value) => Some(value),
            _ => {
                self.push_error(key, reason);
                None
            }
        }
    }

    /// Consume the validator and return either `Ok(())` or all of the collected
    /// problems.
    pub fn finish(self) -> Result<(), ConfigValidationError> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigValidationError {
                module: self.module,
                errors: self.errors,
            })
        }
    }

    fn push_error(&mut self, key: &'static str, message: impl Into<String>) {
        self.errors.push(ConfigError {
            key,
            message: message.into(),
        });
    }
}

/// Error raised when one or more environment variables fail validation.
#[derive(Debug)]
pub struct ConfigValidationError {
    module: String,
    errors: Vec<ConfigError>,
}

impl ConfigValidationError {
    /// Module name whose config failed validation.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Detailed error rows.
    pub fn errors(&self) -> &[ConfigError] {
        &self.errors
    }
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Config validation failed for module {}:", self.module)?;

        let name_header = "NAME";
        let message_header = "MESSAGE";
        let name_width = self
            .errors
            .iter()
            .map(|err| err.key.len())
            .chain(std::iter::once(name_header.len()))
            .max()
            .unwrap_or(name_header.len());
        let message_width = self
            .errors
            .iter()
            .map(|err| err.message.len())
            .chain(std::iter::once(message_header.len()))
            .max()
            .unwrap_or(message_header.len());

        let border = format!(
            "+{}+{}+",
            "-".repeat(name_width + 2),
            "-".repeat(message_width + 2)
        );

        writeln!(f, "{}", border)?;
        writeln!(
            f,
            "| {:<name_width$} | {:<message_width$} |",
            name_header,
            message_header,
            name_width = name_width,
            message_width = message_width
        )?;
        writeln!(f, "{}", border)?;

        for error in &self.errors {
            writeln!(
                f,
                "| {:<name_width$} | {:<message_width$} |",
                error.key,
                error.message,
                name_width = name_width,
                message_width = message_width
            )?;
        }

        write!(f, "{}", border)
    }
}

impl std::error::Error for ConfigValidationError {}

/// Single config problem (key + message).
#[derive(Debug)]
pub struct ConfigError {
    /// Environment variable key.
    pub key: &'static str,
    /// Human readable reason the value failed validation.
    pub message: String,
}

/// Represents an optional value that may fall back to a default.
#[derive(Debug)]
pub struct OptionalValue {
    value: Option<String>,
}

impl OptionalValue {
    /// Use `default` when the environment variable was not provided.
    pub fn or_default(self, default: impl Into<String>) -> String {
        self.value.unwrap_or_else(|| default.into())
    }

    /// Use the closure when the environment variable was not provided.
    pub fn unwrap_or_else(self, default: impl FnOnce() -> String) -> String {
        self.value.unwrap_or_else(default)
    }

    /// Inspect whether the variable was set.
    pub fn present(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

/// Result of parsing a required value.
#[derive(Debug)]
pub struct ParsedValue<T> {
    value: Option<T>,
}

impl<T> ParsedValue<T> {
    /// Use `default` when parsing failed or the value was missing.
    pub fn or_default(self, default: impl FnOnce() -> T) -> T {
        self.value.unwrap_or_else(default)
    }

    /// Use `default` when parsing failed or the value was missing.
    pub fn unwrap_or(self, default: T) -> T {
        self.or_default(|| default)
    }

    /// Convert to an `Option` to inspect whether parsing succeeded.
    pub fn into_option(self) -> Option<T> {
        self.value
    }
}

enum EnvValue {
    Present(String),
    Empty,
    Missing,
}

impl EnvValue {
    fn into_option(self) -> Option<String> {
        match self {
            EnvValue::Present(value) => Some(value),
            _ => None,
        }
    }
}

fn load_env(key: &'static str) -> EnvValue {
    match env::var(key) {
        Ok(value) => {
            if value.trim().is_empty() {
                EnvValue::Empty
            } else {
                EnvValue::Present(value)
            }
        }
        Err(_) => EnvValue::Missing,
    }
}

fn type_name<T>() -> &'static str {
    std::any::type_name::<T>()
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    // Each test gets unique env var names via atomic counter to prevent
    // race conditions when tests run in parallel.
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_key(prefix: &str) -> &'static str {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let s = format!("CV_TEST_{prefix}_{id}");
        Box::leak(s.into_boxed_str())
    }

    #[test]
    fn require_reports_missing_values() {
        let key = unique_key("REQ");
        let mut validator = ConfigValidator::new("require");
        assert!(validator.require(key).is_none());

        let err = validator.finish().unwrap_err();
        assert!(err.to_string().contains(&key));
    }

    #[test]
    fn optional_defaults_when_absent() {
        let key = unique_key("OPT");
        let mut validator = ConfigValidator::new("optional");
        let value = validator.optional(key).or_default("42");
        assert_eq!(value, "42");
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn require_parse_reports_errors_and_returns_default() {
        let key = unique_key("PARSE");
        unsafe { env::set_var(key, "not-a-number"); }

        let mut validator = ConfigValidator::new("parse");
        let port = validator.require_parse::<u16>(key).unwrap_or(8090);
        assert_eq!(port, 8090);

        let err = validator.finish().unwrap_err();
        assert!(err.to_string().contains("u16"));
        assert!(err.to_string().contains("not-a-number"));

        unsafe { env::remove_var(key); }
    }

    #[test]
    fn require_when_condition_true_and_missing_reports_error() {
        let key = unique_key("COND");
        let mut validator = ConfigValidator::new("conditional");
        let value = validator.require_when(
            key,
            || true,
            "CONDITIONAL_TEST is required when the flag is true",
        );
        assert!(value.is_none());

        let err = validator.finish().unwrap_err();
        assert!(err.to_string().contains(&key));
    }

    #[test]
    fn require_when_condition_false_does_not_error() {
        let key = unique_key("COND");
        let mut validator = ConfigValidator::new("conditional-false");
        let value = validator.require_when(
            key,
            || false,
            "should not fire",
        );
        assert!(value.is_none());
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn require_reports_empty_value() {
        let key = unique_key("REQ");
        unsafe { env::set_var(key, "  "); }

        let mut validator = ConfigValidator::new("empty");
        assert!(validator.require(key).is_none());

        let err = validator.finish().unwrap_err();
        assert!(
            err.to_string().contains("empty"),
            "Expected 'empty' in error, got: {err}"
        );
        unsafe { env::remove_var(key); }
    }

    #[test]
    fn finish_collects_multiple_errors() {
        let req_key = unique_key("REQ");
        let parse_key = unique_key("PARSE");
        let mut validator = ConfigValidator::new("multi");
        validator.require(req_key);
        validator.require_parse::<u16>(parse_key);

        let err = validator.finish().unwrap_err();
        assert_eq!(err.errors().len(), 2, "Expected 2 errors, got: {err}");
        assert!(err.to_string().contains(&req_key));
        assert!(err.to_string().contains(&parse_key));
    }

    #[test]
    fn optional_parse_returns_value_when_valid() {
        let key = unique_key("PARSE");
        unsafe { env::set_var(key, "8092"); }

        let mut validator = ConfigValidator::new("opt-parse-ok");
        let port = validator.optional_parse::<u16>(key).unwrap_or(9999);
        assert_eq!(port, 8092);
        assert!(validator.finish().is_ok());

        unsafe { env::remove_var(key); }
    }

    #[test]
    fn optional_parse_uses_default_when_absent() {
        let key = unique_key("PARSE");
        let mut validator = ConfigValidator::new("opt-parse-absent");
        let port = validator.optional_parse::<u16>(key).unwrap_or(8092);
        assert_eq!(port, 8092);
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn optional_parse_reports_error_on_bad_value() {
        let key = unique_key("PARSE");
        unsafe { env::set_var(key, "notanumber"); }

        let mut validator = ConfigValidator::new("opt-parse-bad");
        let port = validator.optional_parse::<u16>(key).unwrap_or(8092);
        assert_eq!(port, 8092);

        let err = validator.finish().unwrap_err();
        assert!(err.to_string().contains("notanumber"));

        unsafe { env::remove_var(key); }
    }

    #[test]
    fn pretty_table_format() {
        let key = unique_key("REQ");
        let mut validator = ConfigValidator::new("pretty");
        validator.require(key);

        let err = validator.finish().unwrap_err();
        let output = err.to_string();
        assert!(
            output.contains('+') && output.contains('|'),
            "Expected table formatting, got:\n{output}"
        );
        assert!(output.contains("Config validation failed for module pretty:"));
    }
}
