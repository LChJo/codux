//! Shared wire and command-line entry points.
use crate::*;
use base64::Engine;
use serde_json::{Value, json};

pub fn dispatch(
    store: &Store,
    project: &str,
    operation: &str,
    payload: &Value,
    actor: Actor,
) -> Result<Value, String> {
    if actor.origin == Origin::Terminal {
        let terminal = store.terminal(
            project,
            actor
                .terminal_id
                .as_deref()
                .ok_or("blackboard.terminal.required")?,
        )?;
        if !terminal.active {
            return Err("blackboard.terminal.inactive".into());
        }
    }
    let component = Component::parse(
        payload
            .get("component")
            .and_then(Value::as_str)
            .unwrap_or("notes"),
    )?;
    let value = match operation {
        "snapshot" => serde_json::to_value(store.snapshot(project)?),
        "poll" => serde_json::to_value(store.snapshot_if_changed(
            project,
            payload.get("knownRevision").and_then(Value::as_u64),
        )?),
        "list" => serde_json::to_value(store.list(project, component)?),
        "read" => serde_json::to_value(store.read(project, component, required(payload, "id")?)?),
        "write" => {
            let id = payload.get("id").and_then(Value::as_str);
            if id.is_some() && actor.origin == Origin::Terminal {
                store.read(project, component, id.unwrap())?;
            }
            let previous = id.and_then(|id| store.read(project, component, id).ok());
            serde_json::to_value(
                store.write(
                    project,
                    component,
                    id,
                    payload
                        .get("title")
                        .and_then(Value::as_str)
                        .or_else(|| previous.as_ref().map(|e| e.title.as_str()))
                        .unwrap_or(""),
                    payload
                        .get("body")
                        .and_then(Value::as_str)
                        .or_else(|| previous.as_ref().map(|e| e.body.as_str()))
                        .unwrap_or(""),
                    payload.get("metadata").cloned().unwrap_or_else(|| {
                        previous
                            .as_ref()
                            .map(|e| e.metadata.clone())
                            .unwrap_or(json!({}))
                    }),
                    actor,
                )?,
            )
        }
        "remove" => serde_json::to_value(store.remove(
            project,
            component,
            required(payload, "id")?,
            actor,
        )?),
        "layout" => {
            if actor.origin != Origin::User {
                return Err("blackboard.component.read_only".into());
            }
            let layout: Layout = serde_json::from_value(
                payload
                    .get("layout")
                    .cloned()
                    .ok_or("blackboard.layout.invalid")?,
            )
            .map_err(|e| e.to_string())?;
            store.save_layout(project, &layout)?;
            Ok(json!({"ok":true}))
        }
        "image.save" => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(required(payload, "bytes")?)
                .map_err(|_| "blackboard.image.invalid")?;
            serde_json::to_value(
                store.save_image(
                    project,
                    payload
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("image.png"),
                    &bytes,
                    actor,
                )?,
            )
        }
        "image.read" => {
            let id = required(payload, "id")?;
            let entry = store.read(project, Component::Images, id)?;
            let bytes = store.read_image(project, id)?;
            Ok(
                json!({"entry":entry,"bytes":base64::engine::general_purpose::STANDARD.encode(bytes),"path":store.image_path(project,id)?}),
            )
        }
        "web.open" => {
            serde_json::to_value(store.open_web(project, required(payload, "url")?, actor)?)
        }
        "web.read" => {
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| store.snapshot(project).ok()?.layout.active_web)
                .ok_or("blackboard.entry.not_found")?;
            let cached = store.read(project, Component::Web, &id)?;
            serde_json::to_value(
                if !cached.body.is_empty()
                    && !payload
                        .get("refresh")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                {
                    cached
                } else {
                    store.fetch_web(project, &id)?
                },
            )
        }
        "web.observe" => {
            if actor.origin != Origin::User {
                return Err("blackboard.component.read_only".into());
            }
            let id = required(payload, "id")?;
            let previous = store.read(project, Component::Web, id)?;
            let mut metadata = previous.metadata.clone();
            if let Some(url) = payload.get("url").and_then(Value::as_str) {
                let url = url::Url::parse(url).map_err(|_| "blackboard.url.invalid")?;
                if !["http", "https"].contains(&url.scheme())
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err("blackboard.url.invalid".into());
                }
                metadata["url"] = url.to_string().into();
            }
            serde_json::to_value(
                store.write(
                    project,
                    Component::Web,
                    Some(id),
                    payload
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or(&previous.title),
                    payload
                        .get("body")
                        .and_then(Value::as_str)
                        .unwrap_or(&previous.body),
                    metadata,
                    Actor::system(
                        previous.terminal_id,
                        previous
                            .metadata
                            .get("automatic")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    ),
                )?,
            )
        }
        "knowledge.import" => {
            if actor.origin != Origin::User {
                return Err("blackboard.component.read_only".into());
            }
            serde_json::to_value(
                store.import_document(
                    project,
                    payload
                        .get("library")
                        .and_then(Value::as_str)
                        .unwrap_or("default"),
                    std::path::Path::new(required(payload, "path")?),
                )?,
            )
        }
        "knowledge.importText" => {
            if actor.origin != Origin::User {
                return Err("blackboard.component.read_only".into());
            }
            serde_json::to_value(
                store.import_text(
                    project,
                    payload
                        .get("library")
                        .and_then(Value::as_str)
                        .unwrap_or("default"),
                    required(payload, "title")?,
                    payload.get("body").and_then(Value::as_str).unwrap_or(""),
                )?,
            )
        }
        "rules.sync" => {
            if actor.origin != Origin::User {
                return Err("blackboard.component.read_only".into());
            }
            let terminal_id = required(payload, "terminalId")?;
            let binding = store.sync_rules(project, terminal_id)?;
            store.write_context(project, terminal_id, None)?;
            serde_json::to_value(binding)
        }
        "terminal.state" => {
            if actor.origin != Origin::User {
                return Err("blackboard.component.read_only".into());
            }
            store.set_terminal_state(
                project,
                required(payload, "terminalId")?,
                payload
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                payload
                    .get("automatic")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )?;
            Ok(json!({"ok":true}))
        }
        _ => return Err("blackboard.operation.invalid".into()),
    };
    value.map_err(|e| e.to_string())
}
fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("blackboard.{key}.required"))
}

pub const CLI_HELP: &str = concat!(
    "codux-blackboard\n",
    "  list [notes|summary|recent|git|images|web|knowledge|rules]\n",
    "  read <component> <id>\n",
    "  add [--title <text>] --body <text>\n",
    "  edit <id> [--title <text>] [--body <text>]\n",
    "  remove <id>\n",
    "  summary add [--title <text>] --body <text>\n",
    "  summary edit <id> [--title <text>] [--body <text>]\n",
    "  summary remove <id>\n",
    "  image save <path> [--title <text>]\n",
    "  image read <id> [--output <path>]\n",
    "  web open <http(s)://url>\n",
    "  web read [id] [--refresh]\n",
    "  context\n",
    "Responses are JSON; context prints its file path. Uses the current Codux terminal project.\n",
    "Shared notes (主动读写, notes): default area for user requests to read or write the blackboard (读取黑板 / 写入黑板), unless another component is explicitly named.\n",
    "Use list notes / read notes <id> / add / edit / remove; list without a component also defaults to notes.\n",
    "Change notes only when explicitly requested or required by explicitly configured project rules. Read back notes to verify writes.\n",
    "Terminal summaries (终端总结, summary): separate milestone history; summary commands do not fulfill requests to write to the blackboard. Do not substitute summaries for empty notes.\n",
    "Rules and knowledge are read-only. File/Git activity updates automatically.\n",
);

#[derive(Default)]
struct CliArguments<'a> {
    positional: Vec<&'a str>,
    values: std::collections::HashMap<&'a str, &'a str>,
    help: bool,
    refresh: bool,
}

fn parse_cli_arguments(args: &[String]) -> Result<CliArguments<'_>, String> {
    let mut parsed = CliArguments::default();
    let mut options = true;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if !options {
            parsed.positional.push(arg);
        } else if arg == "--" {
            options = false;
        } else if matches!(arg, "--title" | "--body" | "--output") {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("codux-blackboard: missing value for {arg}"))?;
            // Option values are opaque text, including --help, other option
            // names and empty strings. Never scan them again as switches.
            parsed.values.insert(arg, value.as_str());
        } else if let Some((key, value)) = arg.split_once('=')
            && matches!(key, "--title" | "--body" | "--output")
        {
            parsed.values.insert(key, value);
        } else if matches!(arg, "--help" | "-h") {
            parsed.help = true;
        } else if arg == "--refresh" {
            parsed.refresh = true;
        } else if arg.starts_with('-') {
            return Err(format!("codux-blackboard: unknown option {arg}"));
        } else {
            parsed.positional.push(arg);
        }
        index += 1;
    }
    Ok(parsed)
}

pub fn run_cli(args: &[String]) -> Result<(), String> {
    run_cli_with_summary_refresh(args, |_, _| Ok(()))
}

pub fn run_cli_with_summary_refresh(
    args: &[String],
    refresh: impl FnOnce(&std::path::Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    let arguments = parse_cli_arguments(args)?;
    if args.is_empty() || arguments.help || arguments.positional.first() == Some(&"help") {
        println!("{CLI_HELP}");
        return Ok(());
    }
    let env = |key: &str| {
        std::env::var(key)
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("codux-blackboard: missing {key}; run from a Codux terminal"))
    };
    let root = env("DMUX_APP_SUPPORT_ROOT")?;
    let project = std::env::var("CODUX_BLACKBOARD_PROJECT_ID")
        .or_else(|_| std::env::var("CODUX_PROJECT_ID"))
        .map_err(|_| "blackboard.project.required")?;
    let terminal = env("CODUX_TERMINAL_ID")?;
    let store = Store::open(&root)?;
    let binding = store.terminal(&project, &terminal)?;
    if !binding.active {
        return Err("blackboard.terminal.inactive".into());
    }
    let arg = |i: usize| {
        arguments
            .positional
            .get(i)
            .copied()
            .ok_or_else(|| CLI_HELP.to_string())
    };
    let flag =
        |key: &str| -> Result<Option<&str>, String> { Ok(arguments.values.get(key).copied()) };
    let summary = arg(0)? == "summary";
    let command = if summary { arg(1)? } else { arg(0)? };
    if summary && !matches!(command, "add" | "edit" | "remove") {
        return Err(CLI_HELP.into());
    }
    let component = if summary { "summary" } else { "notes" };
    let id_index = if summary { 2 } else { 1 };
    let (operation, payload) = match command {
        "list" => (
            "list",
            json!({"component":arguments.positional.get(1).copied().unwrap_or("notes")}),
        ),
        "read" => ("read", json!({"component":arg(1)?,"id":arg(2)?})),
        "add" => {
            let body = flag("--body")?.ok_or("blackboard.content.required")?;
            (
                "write",
                json!({"component":component,"title":flag("--title")?.unwrap_or(""),"body":body,
                    "metadata": if summary { json!({"terminalInstanceId":std::env::var("DMUX_SESSION_INSTANCE_ID").ok()}) } else { json!({}) }}),
            )
        }
        "edit" => {
            let mut value = json!({"component":component,"id":arg(id_index)?});
            if let Some(v) = flag("--title")? {
                value["title"] = v.into();
            }
            if let Some(v) = flag("--body")? {
                value["body"] = v.into();
            }
            ("write", value)
        }
        "remove" => ("remove", json!({"component":component,"id":arg(id_index)?})),
        "image" => match arg(1)? {
            "save" => {
                let path = std::path::Path::new(arg(2)?);
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                (
                    "image.save",
                    json!({"title":flag("--title")?.map(str::to_string).unwrap_or_else(||path.file_name().unwrap_or_default().to_string_lossy().into_owned()),"bytes":base64::engine::general_purpose::STANDARD.encode(bytes)}),
                )
            }
            "read" => ("image.read", json!({"id":arg(2)?})),
            _ => return Err(CLI_HELP.into()),
        },
        "web" => match arg(1)? {
            "open" => ("web.open", json!({"url":arg(2)?})),
            "read" => (
                "web.read",
                json!({"id":arguments.positional.get(2),"refresh":arguments.refresh}),
            ),
            _ => return Err(CLI_HELP.into()),
        },
        "context" => {
            let path = store.write_context(
                &project,
                &terminal,
                std::env::var("DMUX_AI_MEMORY_PROMPT_FILE")
                    .ok()
                    .as_deref()
                    .map(std::path::Path::new),
            )?;
            println!("{}", path.display());
            return Ok(());
        }
        _ => return Err(CLI_HELP.into()),
    };
    if summary && command == "add" {
        refresh(std::path::Path::new(&root), &project)?;
    }
    let mut value = dispatch(
        &store,
        &project,
        operation,
        &payload,
        Actor::terminal(&terminal),
    )?;
    if operation == "image.read"
        && let Some(output) = flag("--output")?
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(value["bytes"].as_str().unwrap_or(""))
            .map_err(|e| e.to_string())?;
        std::fs::write(output, bytes).map_err(|e| e.to_string())?;
        value.as_object_mut().unwrap().remove("bytes");
        value["output"] = output.into();
    }
    if operation == "image.read" {
        value.as_object_mut().unwrap().remove("bytes");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}
