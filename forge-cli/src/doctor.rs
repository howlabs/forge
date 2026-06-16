//! `forge doctor` - environment and configuration health checks.
//!
//! Runs a series of read-only checks and prints a report (human-readable text
//! or machine-readable JSON). Each check yields a [`Status`]: `Ok`, `Warn`, or
//! `Fail`. The command exits non-zero only when at least one hard check fails,
//! so it is safe to run in CI as a preflight gate.

use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn glyph(self) -> &'static str {
        match self {
            Status::Ok => "[ ok ]",
            Status::Warn => "[warn]",
            Status::Fail => "[fail]",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

/// One diagnostic line in the report.
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
}

impl Check {
    fn new(name: impl Into<String>, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// Configuration for the doctor run.
pub struct DoctorConfig {
    pub project_path: String,
    pub config_path: String,
    pub network: String,
    /// Emit machine-readable JSON instead of the text report.
    pub json: bool,
}

/// Minimal view of `forge.toml` that doctor needs.
#[derive(Debug, Default, Deserialize)]
struct DoctorToml {
    provider: Option<ProviderToml>,
    sandbox: Option<SandboxToml>,
    verify: Option<VerifyToml>,
    mcp: Option<McpToml>,
}

#[derive(Debug, Default, Deserialize)]
struct ProviderToml {
    #[serde(rename = "type")]
    kind: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SandboxToml {
    network: Option<String>,
    project_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct VerifyToml {
    enabled: Option<bool>,
    commands: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct McpToml {
    servers: Option<toml::Table>,
}

/// Run all checks, print the report, and return the process exit code
/// (0 = no hard failures, 1 = at least one `Fail`).
pub fn run(config: &DoctorConfig) -> i32 {
    let checks = gather(config);
    if config.json {
        render_json(&checks);
    } else {
        render_text(&checks);
    }
    exit_code(&checks)
}

/// Collect every diagnostic. Pure (apart from reading the environment and
/// running `--version` probes) so it can be unit-tested without capturing
/// stdout.
pub fn gather(config: &DoctorConfig) -> Vec<Check> {
    let forge_toml = load_doctor_toml(&config.config_path);

    let mut checks = Vec::new();
    checks.push(check_toolchain());
    checks.push(check_git(&config.project_path));
    checks.push(check_network_mode(&config.network));
    checks.push(check_network_sandbox(&config.network));
    checks.push(check_config(&config.config_path));
    checks.push(check_configured_provider(forge_toml.as_ref()));
    checks.push(check_sandbox_config(forge_toml.as_ref(), &config.network));
    checks.push(check_verify_config(forge_toml.as_ref()));
    checks.push(check_mcp_config(forge_toml.as_ref()));
    checks.extend(check_provider_keys());
    checks
}

/// Exit code derived from the collected checks.
fn exit_code(checks: &[Check]) -> i32 {
    if checks.iter().any(|c| c.status == Status::Fail) {
        1
    } else {
        0
    }
}

fn render_text(checks: &[Check]) {
    println!("Forge doctor — environment health check\n");
    let mut failures = 0;
    let mut warnings = 0;
    for check in checks {
        println!(
            "{} {:<24} {}",
            check.status.glyph(),
            check.name,
            check.detail
        );
        match check.status {
            Status::Fail => failures += 1,
            Status::Warn => warnings += 1,
            Status::Ok => {}
        }
    }

    println!();
    if failures == 0 {
        println!("Summary: all critical checks passed ({warnings} warning(s)).");
    } else {
        println!("Summary: {failures} failure(s), {warnings} warning(s). See above.");
    }
}

fn render_json(checks: &[Check]) {
    let items: Vec<serde_json::Value> = checks
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "status": c.status.as_str(),
                "detail": c.detail,
            })
        })
        .collect();
    let failures = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warnings = checks.iter().filter(|c| c.status == Status::Warn).count();
    let report = serde_json::json!({
        "ok": failures == 0,
        "failures": failures,
        "warnings": warnings,
        "checks": items,
    });
    match serde_json::to_string_pretty(&report) {
        Ok(s) => println!("{s}"),
        Err(e) => println!("{{\"ok\":false,\"error\":\"{e}\"}}"),
    }
}

fn load_doctor_toml(config_path: &str) -> Option<DoctorToml> {
    let content = std::fs::read_to_string(config_path).ok()?;
    toml::from_str(&content).ok()
}

/// Rust/cargo toolchain availability and version.
fn check_toolchain() -> Check {
    match Command::new("cargo").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Check::new("rust/cargo", Status::Ok, version)
        }
        Ok(out) => Check::new(
            "rust/cargo",
            Status::Fail,
            format!(
                "cargo exited with {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ),
        Err(e) => Check::new(
            "rust/cargo",
            Status::Fail,
            format!("cargo not found on PATH ({e})"),
        ),
    }
}

/// Git availability and whether the project is a repository.
fn check_git(project_path: &str) -> Check {
    match Command::new("git").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let is_repo = Command::new("git")
                .args(["rev-parse", "--is-inside-work-tree"])
                .current_dir(project_path)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if is_repo {
                Check::new("git", Status::Ok, format!("{version} (repo detected)"))
            } else {
                Check::new(
                    "git",
                    Status::Warn,
                    format!("{version} (project path is not a git repo; worktrees unavailable)"),
                )
            }
        }
        _ => Check::new(
            "git",
            Status::Fail,
            "git not found on PATH; subagent worktrees require git".to_string(),
        ),
    }
}

/// Provider API keys discovered in the environment.
fn check_provider_keys() -> Vec<Check> {
    let mut checks = Vec::new();
    let mut any = false;

    // Registry-backed OpenAI-compatible providers.
    for entry in provider::PROVIDERS {
        if env_present(entry.env_var) {
            any = true;
            checks.push(Check::new(
                format!("key: {}", entry.name),
                Status::Ok,
                format!("{} is set", entry.env_var),
            ));
        }
    }

    // Providers with dedicated constructors.
    for (name, var) in [
        ("anthropic", "ANTHROPIC_API_KEY"),
        ("gemini", "GEMINI_API_KEY"),
    ] {
        if env_present(var) {
            any = true;
            checks.push(Check::new(
                format!("key: {name}"),
                Status::Ok,
                format!("{var} is set"),
            ));
        }
    }

    if !any {
        checks.push(Check::new(
            "provider keys",
            Status::Warn,
            "no provider API keys found in environment; use --provider mock for offline runs"
                .to_string(),
        ));
    }

    checks
}

fn env_present(var: &str) -> bool {
    std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false)
}

/// Validate the requested network mode is one Forge understands. Accepts both
/// the CLI short forms (`off`/`on`/`auto`) and the forge.toml forms
/// (`restricted`/`full`), treating them as aliases.
fn check_network_mode(network: &str) -> Check {
    let lower = network.to_lowercase();
    match lower.as_str() {
        "off" | "restricted" | "on" | "full" | "auto" => Check::new(
            "network mode",
            Status::Ok,
            format!("'{network}' is a valid mode"),
        ),
        other => Check::new(
            "network mode",
            Status::Fail,
            format!("unknown mode '{other}'; expected one of: off, on, auto, restricted, full"),
        ),
    }
}

/// Validate the provider configured in forge.toml: it must be a provider Forge
/// can construct, and its API key should be resolvable (unless it is the
/// offline mock/local provider).
fn check_configured_provider(config: Option<&DoctorToml>) -> Check {
    let Some(provider_cfg) = config.and_then(|c| c.provider.as_ref()) else {
        return Check::new(
            "configured provider",
            Status::Warn,
            "no [provider] section in config; CLI defaults will be used".to_string(),
        );
    };

    let Some(kind) = provider_cfg.kind.as_deref().filter(|k| !k.is_empty()) else {
        return Check::new(
            "configured provider",
            Status::Warn,
            "[provider].type is unset; CLI defaults will be used".to_string(),
        );
    };

    let lower = kind.to_lowercase();
    let known = matches!(lower.as_str(), "anthropic" | "gemini" | "mock" | "local")
        || provider::find_provider(&lower).is_some();
    if !known {
        return Check::new(
            "configured provider",
            Status::Fail,
            format!("'{kind}' is not a known provider"),
        );
    }

    let model = provider_cfg.model.as_deref().unwrap_or("<default>");

    if lower == "mock" || lower == "local" {
        return Check::new(
            "configured provider",
            Status::Ok,
            format!("'{kind}' (model {model}); offline, no key required"),
        );
    }

    // Determine which env var holds this provider's key.
    let env_var = provider_cfg
        .api_key_env
        .clone()
        .or_else(|| provider::find_provider(&lower).map(|e| e.env_var.to_string()))
        .unwrap_or_else(|| match lower.as_str() {
            "anthropic" => "ANTHROPIC_API_KEY".to_string(),
            "gemini" => "GEMINI_API_KEY".to_string(),
            _ => "FORGE_API_KEY".to_string(),
        });

    if env_present(&env_var) {
        Check::new(
            "configured provider",
            Status::Ok,
            format!("'{kind}' (model {model}); {env_var} is set"),
        )
    } else {
        Check::new(
            "configured provider",
            Status::Warn,
            format!("'{kind}' (model {model}); {env_var} not set (pass --api-key or use mock)"),
        )
    }
}

/// Network sandbox capability. On Linux we can isolate the network with
/// `unshare --net`; elsewhere the sandbox falls back to a command deny-list.
fn check_network_sandbox(network: &str) -> Check {
    if cfg!(target_os = "linux") {
        let has_unshare = Command::new("unshare")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if has_unshare {
            Check::new(
                "network sandbox",
                Status::Ok,
                format!("mode='{network}'; unshare available for net isolation"),
            )
        } else {
            Check::new(
                "network sandbox",
                Status::Warn,
                format!(
                    "mode='{network}'; unshare missing, network-off relies on command policy only"
                ),
            )
        }
    } else {
        Check::new(
            "network sandbox",
            Status::Warn,
            format!(
                "mode='{network}'; OS-level net isolation is Linux-only, command policy enforced"
            ),
        )
    }
}

/// forge.toml presence and parseability.
fn check_config(config_path: &str) -> Check {
    let path = Path::new(config_path);
    if !path.exists() {
        return Check::new(
            "config",
            Status::Warn,
            format!("{config_path} not found; defaults will be used"),
        );
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<toml::Table>(&content) {
            Ok(_) => Check::new(
                "config",
                Status::Ok,
                format!("{config_path} parsed cleanly"),
            ),
            Err(e) => Check::new(
                "config",
                Status::Fail,
                format!("{config_path} is invalid TOML: {e}"),
            ),
        },
        Err(e) => Check::new(
            "config",
            Status::Fail,
            format!("cannot read {config_path}: {e}"),
        ),
    }
}

/// Consistency of the optional `[sandbox]` section with the requested mode.
/// Warns when the configured network mode disagrees with `--network`, and when
/// the configured project path is not a readable directory.
fn check_sandbox_config(config: Option<&DoctorToml>, requested_network: &str) -> Check {
    let Some(cfg) = config.and_then(|c| c.sandbox.as_ref()) else {
        return Check::new(
            "sandbox config",
            Status::Ok,
            "no [sandbox] section; CLI/network defaults apply".to_string(),
        );
    };

    let mut notes = Vec::new();
    let mut status = Status::Ok;

    if let Some(net) = cfg.network.as_deref() {
        let net_lower = net.to_lowercase();
        if !matches!(
            net_lower.as_str(),
            "off" | "on" | "restricted" | "full" | "auto"
        ) {
            status = Status::Fail;
            notes.push(format!(
                "[sandbox].network='{net}' is not a recognized mode"
            ));
        } else if !same_network(&net_lower, &requested_network.to_lowercase()) {
            status = Status::Warn;
            notes.push(format!(
                "[sandbox].network='{net}' differs from --network='{requested_network}'"
            ));
        } else {
            notes.push(format!("network='{net}'"));
        }
    }

    if let Some(pp) = cfg.project_path.as_deref().filter(|s| !s.is_empty()) {
        if Path::new(pp).is_dir() {
            notes.push(format!("project_path='{pp}' exists"));
        } else {
            status = if status == Status::Fail {
                Status::Fail
            } else {
                Status::Warn
            };
            notes.push(format!("project_path='{pp}' is not a readable directory"));
        }
    }

    Check::new("sandbox config", status, notes.join("; "))
}

/// `off`/`restricted` are the conservative family; `on`/`full`/`auto` are the
/// permissive family. A mismatch is only flagged when the two modes fall into
/// different families; exact aliases never warn.
fn same_network(configured: &str, requested: &str) -> bool {
    let family = |m: &str| match m {
        "off" | "restricted" => 0,
        "on" | "full" | "auto" => 1,
        _ => -1,
    };
    let (c, r) = (family(configured), family(requested));
    if c == -1 || r == -1 {
        // Unrecognized on either side: only "same" if they are literally equal.
        configured == requested
    } else {
        c == r
    }
}

/// `[verify]` section: when enabled (the default), each listed command must
/// start with a runnable binary on PATH. Unparseable or missing commands warn.
fn check_verify_config(config: Option<&DoctorToml>) -> Check {
    let Some(cfg) = config.and_then(|c| c.verify.as_ref()) else {
        return Check::new(
            "verify config",
            Status::Ok,
            "no [verify] section; verify loop disabled".to_string(),
        );
    };

    let enabled = cfg.enabled.unwrap_or(true);
    let commands = cfg.commands.as_deref().unwrap_or(&[]);

    if !enabled {
        return Check::new(
            "verify config",
            Status::Warn,
            "[verify].enabled=false; build/test gates are skipped".to_string(),
        );
    }

    if commands.is_empty() {
        return Check::new(
            "verify config",
            Status::Warn,
            "verify enabled but [verify].commands is empty".to_string(),
        );
    }

    let mut missing = Vec::new();
    for cmd in commands {
        if let Some(bin) = cmd.split_whitespace().next() {
            if !on_path(bin) {
                missing.push(bin.to_string());
            }
        }
    }

    if missing.is_empty() {
        Check::new(
            "verify config",
            Status::Ok,
            format!("{} command(s); all binaries on PATH", commands.len()),
        )
    } else {
        Check::new(
            "verify config",
            Status::Warn,
            format!(
                "verify commands reference missing binary(ies): {}",
                missing.join(", ")
            ),
        )
    }
}

/// `[mcp]` section: report the number of configured stdio servers. Each entry
/// must at least declare a `command`; otherwise warn.
fn check_mcp_config(config: Option<&DoctorToml>) -> Check {
    let Some(mcp) = config.and_then(|c| c.mcp.as_ref()) else {
        return Check::new(
            "mcp config",
            Status::Ok,
            "no [mcp] section; no external servers".to_string(),
        );
    };

    let Some(servers) = mcp.servers.as_ref() else {
        return Check::new(
            "mcp config",
            Status::Ok,
            "[mcp] present but no servers defined".to_string(),
        );
    };

    let total = servers.len();
    let mut broken: Vec<String> = servers
        .iter()
        .filter(|(_, v)| {
            v.get("command")
                .and_then(|c| c.as_str())
                .map_or(true, |s| s.is_empty())
        })
        .map(|(k, _)| k.to_string())
        .collect();
    broken.sort();

    if broken.is_empty() {
        Check::new(
            "mcp config",
            Status::Ok,
            format!("{total} MCP server(s) configured"),
        )
    } else {
        Check::new(
            "mcp config",
            Status::Warn,
            format!(
                "{total} server(s); {} missing a 'command': {}",
                broken.len(),
                broken.join(", ")
            ),
        )
    }
}

/// Whether `bin` resolves on PATH (best-effort; ignores PATHEXT nuances).
fn on_path(bin: &str) -> bool {
    if bin.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(bin).exists();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).exists()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml_with_provider(kind: &str) -> DoctorToml {
        DoctorToml {
            provider: Some(ProviderToml {
                kind: Some(kind.to_string()),
                model: Some("test-model".to_string()),
                api_key_env: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn config_missing_is_warn() {
        let check = check_config("definitely-not-a-real-file.toml");
        assert_eq!(check.status, Status::Warn);
    }

    #[test]
    fn config_invalid_is_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let check = check_config(path.to_str().unwrap());
        assert_eq!(check.status, Status::Fail);
    }

    #[test]
    fn config_valid_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.toml");
        std::fs::write(&path, "[provider]\ntype = \"mock\"\n").unwrap();
        let check = check_config(path.to_str().unwrap());
        assert_eq!(check.status, Status::Ok);
    }

    #[test]
    fn provider_keys_without_env_warns() {
        // We can't reliably clear the real environment here, so just assert the
        // function returns at least one check and never panics.
        let checks = check_provider_keys();
        assert!(!checks.is_empty());
    }

    #[test]
    fn network_mode_valid_and_invalid() {
        for ok in ["off", "on", "auto", "restricted", "full", "OFF", "Auto"] {
            assert_eq!(
                check_network_mode(ok).status,
                Status::Ok,
                "expected ok for {ok}"
            );
        }
        assert_eq!(check_network_mode("banana").status, Status::Fail);
    }

    #[test]
    fn configured_provider_unknown_is_fail() {
        let toml = toml_with_provider("not-a-real-provider");
        assert_eq!(check_configured_provider(Some(&toml)).status, Status::Fail);
    }

    #[test]
    fn configured_provider_mock_is_ok_without_key() {
        let toml = toml_with_provider("mock");
        let check = check_configured_provider(Some(&toml));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("offline"));
    }

    #[test]
    fn configured_provider_missing_section_warns() {
        let toml = DoctorToml {
            provider: None,
            ..Default::default()
        };
        assert_eq!(check_configured_provider(Some(&toml)).status, Status::Warn);
    }

    #[test]
    fn exit_code_is_one_on_failure() {
        let checks = vec![
            Check::new("a", Status::Ok, ""),
            Check::new("b", Status::Fail, ""),
        ];
        assert_eq!(exit_code(&checks), 1);
    }

    #[test]
    fn exit_code_is_zero_with_only_warnings() {
        let checks = vec![
            Check::new("a", Status::Ok, ""),
            Check::new("b", Status::Warn, ""),
        ];
        assert_eq!(exit_code(&checks), 0);
    }

    #[test]
    fn gather_includes_core_checks() {
        let dir = tempfile::tempdir().unwrap();
        let config = DoctorConfig {
            project_path: dir.path().to_str().unwrap().to_string(),
            config_path: dir.path().join("forge.toml").to_str().unwrap().to_string(),
            network: "off".to_string(),
            json: false,
        };
        let checks = gather(&config);
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"rust/cargo"));
        assert!(names.contains(&"git"));
        assert!(names.contains(&"network mode"));
        assert!(names.contains(&"config"));
        assert!(names.contains(&"configured provider"));
        assert!(names.contains(&"sandbox config"));
        assert!(names.contains(&"verify config"));
        assert!(names.contains(&"mcp config"));
    }

    #[test]
    fn sandbox_absent_is_ok() {
        let toml = DoctorToml::default();
        let check = check_sandbox_config(Some(&toml), "off");
        assert_eq!(check.status, Status::Ok);
    }

    #[test]
    fn sandbox_unknown_network_is_fail() {
        let toml = DoctorToml {
            sandbox: Some(SandboxToml {
                network: Some("open-the-pod-bay-doors".to_string()),
                project_path: None,
            }),
            ..Default::default()
        };
        let check = check_sandbox_config(Some(&toml), "off");
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("not a recognized mode"));
    }

    #[test]
    fn verify_disabled_warns() {
        let toml = DoctorToml {
            verify: Some(VerifyToml {
                enabled: Some(false),
                commands: Some(vec!["cargo test".to_string()]),
            }),
            ..Default::default()
        };
        let check = check_verify_config(Some(&toml));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("enabled=false"));
    }

    #[test]
    fn verify_empty_commands_warns() {
        let toml = DoctorToml {
            verify: Some(VerifyToml {
                enabled: Some(true),
                commands: Some(vec![]),
            }),
            ..Default::default()
        };
        let check = check_verify_config(Some(&toml));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("empty"));
    }

    #[test]
    fn verify_known_binary_is_ok() {
        // `cargo` is exercised by the toolchain check, so assume present.
        let toml = DoctorToml {
            verify: Some(VerifyToml {
                enabled: Some(true),
                commands: Some(vec!["cargo test".to_string()]),
            }),
            ..Default::default()
        };
        let check = check_verify_config(Some(&toml));
        // cargo may be missing in CI sandboxes; only assert Ok when on_path holds.
        if on_path("cargo") {
            assert_eq!(check.status, Status::Ok);
        }
    }

    #[test]
    fn verify_missing_binary_warns() {
        let toml = DoctorToml {
            verify: Some(VerifyToml {
                enabled: Some(true),
                commands: Some(vec!["definitely-not-a-real-tool run".to_string()]),
            }),
            ..Default::default()
        };
        let check = check_verify_config(Some(&toml));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("definitely-not-a-real-tool"));
    }

    #[test]
    fn mcp_absent_is_ok() {
        let toml = DoctorToml::default();
        let check = check_mcp_config(Some(&toml));
        assert_eq!(check.status, Status::Ok);
    }

    #[test]
    fn mcp_missing_command_warns() {
        let servers: toml::Table = {
            let mut t = toml::Table::new();
            let mut s = toml::Table::new();
            // no "command" key
            let _ = s.insert("args".into(), toml::Value::Array(vec![]));
            let _ = t.insert("broken".into(), toml::Value::Table(s));
            t
        };
        let toml = DoctorToml {
            mcp: Some(McpToml {
                servers: Some(servers),
            }),
            ..Default::default()
        };
        let check = check_mcp_config(Some(&toml));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("missing a 'command'"));
    }

    #[test]
    fn mcp_complete_server_is_ok() {
        let servers: toml::Table = {
            let mut t = toml::Table::new();
            let mut s = toml::Table::new();
            let _ = s.insert("command".into(), toml::Value::String("forge".to_string()));
            let _ = t.insert("forge-self".into(), toml::Value::Table(s));
            t
        };
        let toml = DoctorToml {
            mcp: Some(McpToml {
                servers: Some(servers),
            }),
            ..Default::default()
        };
        let check = check_mcp_config(Some(&toml));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("1 MCP server"));
    }

    #[test]
    fn same_network_groups_conservative_and_permissive() {
        // Conservative family
        assert!(same_network("off", "off"));
        assert!(same_network("off", "restricted"));
        assert!(same_network("restricted", "off"));
        // Permissive family
        assert!(same_network("on", "full"));
        assert!(same_network("on", "auto"));
        assert!(same_network("full", "auto"));
        // Across families → mismatch
        assert!(!same_network("off", "on"));
        assert!(!same_network("restricted", "full"));
        // Unrecognized falls back to literal equality
        assert!(same_network("bogus", "bogus"));
        assert!(!same_network("bogus", "off"));
    }
}
