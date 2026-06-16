use anyhow::{Context, Result};
use forge_core::{EventLoop, LoopEvent, ToolPolicy};
use provider::ModelProvider;
use sandbox::Sandbox;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use verify::BuildVerifier;

#[derive(Debug, Clone)]
pub struct PlainReplConfig {
    pub provider_name: String,
    pub model: String,
    pub project_path: PathBuf,
    pub config_path: PathBuf,
    pub max_verify_retries: usize,
    pub resume_session: Option<String>,
}

#[derive(Debug, Default)]
struct ReplState {
    approve_session: bool,
    cost: CostEstimate,
    patches: Vec<PatchTransaction>,
}

#[derive(Debug, Default, Deserialize)]
struct PermissionsToml {
    permissions: Option<PermissionSection>,
}

#[derive(Debug, Default, Deserialize)]
struct PermissionSection {
    /// Start plain REPL sessions with write/edit tools approved.
    allow_writes: Option<bool>,
    /// Start plain REPL sessions with command execution approved.
    allow_commands: Option<bool>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CostEstimate {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    tasks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PatchTransaction {
    id: usize,
    path: String,
    old_text: String,
    new_text: String,
    applied_at: u64,
}

#[derive(Debug, Default, Clone)]
struct SessionSummary {
    user_inputs: usize,
    assistant_messages: usize,
    patches: Vec<PatchTransaction>,
    latest_cost: CostEstimate,
}

impl SessionSummary {
    fn to_repl_state(&self) -> ReplState {
        ReplState {
            approve_session: false,
            cost: self.latest_cost.clone(),
            patches: self.patches.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SessionRecord {
    SessionStarted {
        session_id: String,
        provider: String,
        model: String,
        project_path: String,
        ts: u64,
    },
    UserInput {
        content: String,
        ts: u64,
    },
    Assistant {
        step: usize,
        content: String,
        ts: u64,
    },
    Tool {
        name: String,
        result: String,
        is_error: bool,
        ts: u64,
    },
    Patch(PatchTransaction),
    Verify {
        passed: bool,
        logs: String,
        ts: u64,
    },
    Cost {
        estimate: CostEstimate,
        ts: u64,
    },
}

pub async fn run_plain_repl<P>(
    provider: P,
    context: context::ContextEngine,
    sandbox: Sandbox,
    cfg: PlainReplConfig,
) -> Result<()>
where
    P: ModelProvider + 'static,
{
    fs::create_dir_all(cfg.project_path.join(".forge/sessions"))?;
    let session_id = cfg.resume_session.clone().unwrap_or_else(new_session_id);
    let session_path = cfg
        .project_path
        .join(".forge/sessions")
        .join(format!("{session_id}.jsonl"));
    let logger = SessionLogger::new(session_path.clone())?;
    let initial_state = if cfg.resume_session.is_some() {
        println!(
            "Resumed session {session_id} from {}",
            session_path.display()
        );
        Some(replay_summary(&session_path)?)
    } else {
        logger.append(&SessionRecord::SessionStarted {
            session_id: session_id.clone(),
            provider: cfg.provider_name.clone(),
            model: cfg.model.clone(),
            project_path: cfg.project_path.display().to_string(),
            ts: now(),
        })?;
        None
    };

    println!("Forge plain REPL v0.120 — session {session_id}");
    println!("Type /help for commands. Safe reads are automatic; writes and commands need /approve session. Transcript: {}", session_path.display());
    println!(
        "Config: {} | max verify retries: {}",
        cfg.config_path.display(),
        cfg.max_verify_retries
    );

    let mut restored_state = initial_state
        .as_ref()
        .map(SessionSummary::to_repl_state)
        .unwrap_or_default();
    let policy = load_permission_policy(&cfg.project_path, &cfg.config_path)?;
    restored_state.approve_session =
        policy.allow_writes.unwrap_or(false) && policy.allow_commands.unwrap_or(false);
    let state = Arc::new(Mutex::new(restored_state));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LoopEvent>();
    let logger_for_events = logger.clone();
    let state_for_events = state.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let record = match event {
                LoopEvent::AssistantMessage { step, content } => {
                    println!("\n{content}\n");
                    SessionRecord::Assistant {
                        step,
                        content,
                        ts: now(),
                    }
                }
                LoopEvent::ToolStarted { name } => {
                    println!("→ tool: {name}");
                    continue;
                }
                LoopEvent::ToolCompleted {
                    name,
                    result,
                    is_error,
                } => {
                    println!(
                        "{} {name}: {}",
                        if is_error { "✗" } else { "✓" },
                        trim_multiline(&result)
                    );
                    SessionRecord::Tool {
                        name,
                        result,
                        is_error,
                        ts: now(),
                    }
                }
                LoopEvent::DiffApplied {
                    path,
                    old_text,
                    new_text,
                } => {
                    let patch = {
                        let mut locked = state_for_events.lock().unwrap();
                        let id = locked.patches.len() + 1;
                        let patch = PatchTransaction {
                            id,
                            path,
                            old_text,
                            new_text,
                            applied_at: now(),
                        };
                        locked.patches.push(patch.clone());
                        patch
                    };
                    println!("✓ patch #{} applied to {}", patch.id, patch.path);
                    SessionRecord::Patch(patch)
                }
                LoopEvent::VerifyResult { passed, logs } => {
                    println!("{} verify", if passed { "✓" } else { "✗" });
                    SessionRecord::Verify {
                        passed,
                        logs,
                        ts: now(),
                    }
                }
            };
            let _ = logger_for_events.append(&record);
        }
    });

    let mut event_loop =
        EventLoop::new(provider, context, sandbox, String::new()).with_observer(tx);

    loop {
        print!("forge> ");
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }
        logger.append(&SessionRecord::UserInput {
            content: input.clone(),
            ts: now(),
        })?;

        if input.starts_with('/') {
            if handle_command(&input, &state, &logger, &cfg).await? {
                break;
            }
            continue;
        }

        let policy = {
            let locked = state.lock().unwrap();
            ToolPolicy {
                allow_writes: locked.approve_session,
                allow_commands: locked.approve_session,
            }
        };
        event_loop = event_loop
            .with_repl_turn(input.clone())
            .with_tool_policy(policy);
        match event_loop.run().await {
            Ok(steps) => {
                let mut locked = state.lock().unwrap();
                locked.cost.tasks += 1;
                locked.cost.prompt_tokens += estimate_tokens(&input);
                locked.cost.completion_tokens += steps as u64 * 128;
                locked.cost.total_tokens =
                    locked.cost.prompt_tokens + locked.cost.completion_tokens;
                logger.append(&SessionRecord::Cost {
                    estimate: locked.cost.clone(),
                    ts: now(),
                })?;
            }
            Err(e) => eprintln!("error: {e:#}"),
        }
    }
    Ok(())
}

fn load_permission_policy(project_path: &Path, config_path: &Path) -> Result<PermissionSection> {
    let mut policy = PermissionSection::default();
    for path in [
        project_path.join("forge.toml"),
        project_path.join(".forge/permissions.toml"),
        config_path.to_path_buf(),
    ] {
        if !path.exists() {
            continue;
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read permissions config {}", path.display()))?;
        if let Ok(parsed) = toml::from_str::<PermissionsToml>(&content) {
            if let Some(section) = parsed.permissions {
                if section.allow_writes.is_some() {
                    policy.allow_writes = section.allow_writes;
                }
                if section.allow_commands.is_some() {
                    policy.allow_commands = section.allow_commands;
                }
            }
        }
    }
    Ok(policy)
}

async fn handle_command(
    input: &str,
    state: &Arc<Mutex<ReplState>>,
    logger: &SessionLogger,
    cfg: &PlainReplConfig,
) -> Result<bool> {
    let mut parts = input.splitn(4, ' ');
    let cmd = parts.next().unwrap_or("");
    match cmd {
        "/help" => print_help(),
        "/model" => println!("provider={} model={}", cfg.provider_name, cfg.model),
        "/cost" => {
            let c = state.lock().unwrap().cost.clone();
            println!(
                "tasks={} prompt≈{} completion≈{} total≈{} tokens",
                c.tasks, c.prompt_tokens, c.completion_tokens, c.total_tokens
            );
        }
        "/approve" => {
            let scope = parts.next().unwrap_or("status");
            let mut s = state.lock().unwrap();
            match scope {
                "session" => {
                    s.approve_session = true;
                    println!("Writes and command execution approved for this session.");
                }
                "off" => {
                    s.approve_session = false;
                    println!("Writes and command execution now require approval again.");
                }
                _ => println!(
                    "approval: safe reads=auto, writes/runs={}",
                    if s.approve_session {
                        "session"
                    } else {
                        "blocked"
                    }
                ),
            }
        }
        "/diff" => {
            let path = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: /diff <path> <old> <new>"))?;
            let old = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: /diff <path> <old> <new>"))?;
            let new = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: /diff <path> <old> <new>"))?;
            let sandbox = Sandbox::new(&cfg.project_path, "off")?;
            match sandbox.preview_diff_edit(path, old, new).await {
                Ok(preview) => println!("{}\n--- {path}\n-{}\n+{}", preview.summary(), old, new),
                Err(e) => println!(
                    "Dry-run validation failed: {e:#}\n--- {path}\n-{}\n+{}",
                    old, new
                ),
            }
            println!("Dry-run only. Ask the agent to apply it or use /approve session first for agent edits.");
        }
        "/undo" => {
            let patch = state.lock().unwrap().patches.pop();
            if let Some(patch) = patch {
                let sandbox = Sandbox::new(&cfg.project_path, "off")?;
                sandbox
                    .diff_edit(&patch.path, &patch.new_text, &patch.old_text)
                    .await?;
                println!("Rolled back patch #{} ({})", patch.id, patch.path);
                logger.append(&SessionRecord::Tool {
                    name: "undo".into(),
                    result: format!("Rolled back patch #{}", patch.id),
                    is_error: false,
                    ts: now(),
                })?;
            } else {
                println!("No patch transaction to undo in this live session.");
            }
        }
        "/verify" => {
            let verifier = BuildVerifier::new();
            let report = forge_core::Verifier::verify(&verifier, &cfg.project_path).await?;
            println!(
                "{} verify in {}ms",
                if report.passed { "✓" } else { "✗" },
                report.duration_ms
            );
            if !report.logs.trim().is_empty() {
                println!("{}", report.logs);
            }
            logger.append(&SessionRecord::Verify {
                passed: report.passed,
                logs: report.logs,
                ts: now(),
            })?;
        }
        "/exit" | "/quit" => return Ok(true),
        _ => println!("Unknown command. Type /help."),
    }
    Ok(false)
}

fn print_help() {
    println!(
        r#"Commands:
  /help                 Show this help
  /model                Show active provider/model
  /cost                 Show approximate per-session token cost counters
  /diff <path> <old> <new>  Preview a small replacement as a dry-run
  /approve [status|session|off]  Approve writes/runs for this session or turn them off
  /undo                 Roll back the latest diff_edit patch seen by this REPL
  /verify               Run the project verify loop and print logs
  /quit                 Exit

Type a normal task to run the agent. Safe reads are auto-approved; write_file,
diff_edit, and run_command require /approve session in plain REPL mode."#
    );
}

#[derive(Clone)]
struct SessionLogger {
    path: PathBuf,
}
impl SessionLogger {
    fn new(path: PathBuf) -> Result<Self> {
        Ok(Self { path })
    }
    fn append(&self, record: &SessionRecord) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, record)?;
        writeln!(file)?;
        Ok(())
    }
}

pub fn resume_session(project_path: &Path, session_id: &str) -> Result<()> {
    let path = project_path
        .join(".forge/sessions")
        .join(format!("{session_id}.jsonl"));
    replay_summary(&path).map(|_| ())
}

pub fn list_sessions(project_path: &Path) -> Result<()> {
    let dir = project_path.join(".forge/sessions");
    if !dir.exists() {
        println!("No sessions found.");
        return Ok(());
    }

    let mut entries = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });

    if entries.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("{:<20} {:<8} {:<8} PATCHES", "SESSION", "INPUTS", "MSGS");
    for entry in entries {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let summary = replay_summary_silent(&path)?;
        println!(
            "{:<20} {:<8} {:<8} {}",
            id,
            summary.user_inputs,
            summary.assistant_messages,
            summary.patches.len()
        );
    }
    Ok(())
}

fn replay_summary_silent(path: &Path) -> Result<SessionSummary> {
    summarize_session(path, false)
}

fn replay_summary(path: &Path) -> Result<SessionSummary> {
    summarize_session(path, true)
}

fn summarize_session(path: &Path, print: bool) -> Result<SessionSummary> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read session {}", path.display()))?;
    let mut summary = SessionSummary::default();
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(record) = serde_json::from_str::<SessionRecord>(line) {
            match record {
                SessionRecord::UserInput { .. } => summary.user_inputs += 1,
                SessionRecord::Assistant { .. } => summary.assistant_messages += 1,
                SessionRecord::Patch(patch) => summary.patches.push(patch),
                SessionRecord::Cost { estimate, .. } => summary.latest_cost = estimate,
                _ => {}
            }
        }
    }
    if print {
        println!(
            "Session {}: {} user inputs, {} assistant messages, {} patches",
            path.display(),
            summary.user_inputs,
            summary.assistant_messages,
            summary.patches.len()
        );
    }
    Ok(summary)
}

fn estimate_tokens(s: &str) -> u64 {
    (s.chars().count() as u64).div_ceil(4).max(1)
}
fn new_session_id() -> String {
    format!("{}", now())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn trim_multiline(s: &str) -> String {
    let s = s.trim();
    if s.len() > 240 {
        let end = s.floor_char_boundary(240);
        format!("{}…", &s[..end])
    } else {
        s.replace('\n', " | ")
    }
}
