/// Controls whether ANSI color codes are emitted in terminal output.
///
/// Respects the [NO_COLOR](https://no-color.org/) standard: if the `NO_COLOR`
/// environment variable is set to any non-empty value, color is disabled
/// regardless of other settings.
///
/// # Usage
/// ```
/// use accelmars_gateway_core::OutputConfig;
///
/// // Typically constructed once at CLI entry, then passed to display functions.
/// let out = OutputConfig::from_env(false); // false = --no-color flag not set
/// let text = out.colorize("running", "\x1b[32m", "\x1b[0m");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct OutputConfig {
    pub color: bool,
}

impl OutputConfig {
    /// Build from environment.
    ///
    /// `no_color_flag`: true if the caller's `--no-color` CLI flag was passed.
    ///
    /// Color is disabled if either:
    /// - `NO_COLOR` env var is set to a non-empty value (no-color.org standard)
    /// - `no_color_flag` is `true`
    pub fn from_env(no_color_flag: bool) -> Self {
        let env_disabled = std::env::var("NO_COLOR")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        Self {
            color: !env_disabled && !no_color_flag,
        }
    }

    /// Wrap `text` in ANSI escape codes when color is enabled.
    ///
    /// When color is disabled, returns `text` unchanged — no ANSI codes emitted.
    ///
    /// # Arguments
    /// - `text` — the string to colorize
    /// - `ansi_open` — opening escape code (e.g., `"\x1b[32m"` for green)
    /// - `ansi_close` — closing escape code (e.g., `"\x1b[0m"` for reset)
    pub fn colorize(&self, text: &str, ansi_open: &str, ansi_close: &str) -> String {
        if self.color {
            format!("{}{}{}", ansi_open, text, ansi_close)
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_env_disables_color() {
        // Set env var, construct, then unset — use a scoped approach
        std::env::set_var("NO_COLOR", "1");
        let out = OutputConfig::from_env(false);
        std::env::remove_var("NO_COLOR");
        assert!(!out.color, "NO_COLOR=1 should disable color");
    }

    #[test]
    fn no_color_flag_disables_color() {
        std::env::remove_var("NO_COLOR"); // ensure not set
        let out = OutputConfig::from_env(true);
        assert!(!out.color, "--no-color flag should disable color");
    }

    #[test]
    fn both_unset_enables_color() {
        std::env::remove_var("NO_COLOR");
        let out = OutputConfig::from_env(false);
        assert!(
            out.color,
            "color should be enabled when neither env nor flag is set"
        );
    }

    #[test]
    fn both_set_disables_color() {
        std::env::set_var("NO_COLOR", "1");
        let out = OutputConfig::from_env(true);
        std::env::remove_var("NO_COLOR");
        assert!(
            !out.color,
            "color should be disabled when both env and flag are set"
        );
    }

    #[test]
    fn colorize_wraps_when_enabled() {
        let out = OutputConfig { color: true };
        let result = out.colorize("running", "\x1b[32m", "\x1b[0m");
        assert_eq!(result, "\x1b[32mrunning\x1b[0m");
    }

    #[test]
    fn colorize_plain_when_disabled() {
        let out = OutputConfig { color: false };
        let result = out.colorize("running", "\x1b[32m", "\x1b[0m");
        assert_eq!(result, "running");
    }
}
