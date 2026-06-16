use anyhow::Result;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ReleaseCheckConfig {
    pub project_path: PathBuf,
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ReleaseCheckReport {
    passed: bool,
    checks: Vec<ReleaseCheck>,
}

#[derive(Debug, Serialize)]
struct ReleaseCheck {
    name: &'static str,
    passed: bool,
    detail: String,
}

pub fn run(config: ReleaseCheckConfig) -> Result<i32> {
    let checks = vec![
        check_workspace_manifest(&config),
        check_license(&config),
        check_model_catalog(),
        check_verify_detection(&config),
        check_context_store_parent(&config),
    ];

    let passed = checks.iter().all(|check| check.passed);
    let report = ReleaseCheckReport { passed, checks };
    if config.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for check in &report.checks {
            println!(
                "{} {} — {}",
                if check.passed { "PASS" } else { "FAIL" },
                check.name,
                check.detail
            );
        }
    }
    Ok(if passed { 0 } else { 1 })
}

fn check_workspace_manifest(config: &ReleaseCheckConfig) -> ReleaseCheck {
    let path = config.project_path.join("Cargo.toml");
    ReleaseCheck {
        name: "workspace manifest",
        passed: path.exists(),
        detail: path.display().to_string(),
    }
}

fn check_license(config: &ReleaseCheckConfig) -> ReleaseCheck {
    let cargo = std::fs::read_to_string(config.project_path.join("Cargo.toml")).unwrap_or_default();
    let passed = cargo.contains("MIT OR Apache-2.0");
    ReleaseCheck {
        name: "license metadata",
        passed,
        detail: if passed {
            "MIT OR Apache-2.0"
        } else {
            "missing expected workspace license"
        }
        .into(),
    }
}

fn check_model_catalog() -> ReleaseCheck {
    let count = provider::MODEL_CATALOG.len();
    ReleaseCheck {
        name: "model catalog",
        passed: count >= 5,
        detail: format!("{count} models registered"),
    }
}

fn check_verify_detection(config: &ReleaseCheckConfig) -> ReleaseCheck {
    let commands = verify::detect_verify_commands(&config.project_path).unwrap_or_default();
    ReleaseCheck {
        name: "verify profile",
        passed: !commands.is_empty(),
        detail: if commands.is_empty() {
            "no commands detected".into()
        } else {
            commands.join(" && ")
        },
    }
}

fn check_context_store_parent(config: &ReleaseCheckConfig) -> ReleaseCheck {
    let forge_dir = config.project_path.join(".forge");
    ReleaseCheck {
        name: "forge state dir",
        passed: forge_dir.exists() || config.project_path.exists(),
        detail: forge_dir.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_catalog_check_passes() {
        assert!(check_model_catalog().passed);
    }
}
