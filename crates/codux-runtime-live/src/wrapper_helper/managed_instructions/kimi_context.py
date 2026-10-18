"""Build a launch-scoped Kimi agent using that installation's own loader."""
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import uuid


def prepare(request):
    from kimi_cli.agentspec import DEFAULT_AGENT_FILE, OKABE_AGENT_FILE, load_agent_spec

    args = request["args"]
    rest, agent, agent_file = [], None, None
    index = 0
    value_options = {
        "--prompt", "-p", "--model", "-m", "--config", "--config-file",
        "--work-dir", "-w", "--add-dir", "--mcp-config", "--mcp-config-file",
        "--skills-dir", "--session", "--resume", "--input-format", "--output-format",
    }
    while index < len(args):
        arg = args[index]
        if arg == "--":
            rest.extend(args[index:])
            break
        if arg in ("--agent", "--agent-file"):
            value = args[index + 1]
            if arg == "--agent":
                agent = value
            else:
                agent_file = value
            index += 2
        elif arg.startswith("--agent="):
            agent = arg.partition("=")[2]
            index += 1
        elif arg.startswith("--agent-file="):
            agent_file = arg.partition("=")[2]
            index += 1
        else:
            rest.append(arg)
            index += 1
            if arg in value_options and index < len(args):
                rest.append(args[index])
                index += 1
    if agent is not None and agent_file is not None:
        raise ValueError("Conflicting agent selectors")
    if agent_file is not None:
        base = Path(agent_file).expanduser().resolve(strict=True)
    elif agent in (None, "default"):
        base = DEFAULT_AGENT_FILE
    elif agent == "okabe":
        base = OKABE_AGENT_FILE
    else:
        raise ValueError("Unsupported built-in agent")
    spec = load_agent_spec(base)
    prompt = Path(spec.system_prompt_path).resolve(strict=True)
    context = Path(request["contextFile"]).resolve(strict=True)
    body = context.read_text(encoding="utf-8-sig")
    if len(body.encode("utf-8")) > 8 * 1024 * 1024:
        raise ValueError("Context too large")

    # Preserve the original loader root, including dynamic includes, macros,
    # imports and extends. Only top-level aliases are needed; never traverse or
    # copy the directory tree, and never write to the user's template folder.
    children = list(prompt.parent.iterdir())
    if len(children) > 4096:
        raise ValueError("Template directory too large")
    managed = Path(tempfile.mkdtemp(prefix="kimi-agent-", dir=context.parent))
    copied = 0
    for child in children:
        target = managed / child.name
        if child.is_dir():
            if os.name == "nt":
                import _winapi
                _winapi.CreateJunction(str(child.resolve()), str(target))
            else:
                target.symlink_to(child.resolve(), target_is_directory=True)
        elif child.is_file():
            try:
                os.link(child, target)
            except OSError:
                copied += child.stat().st_size
                if copied > 32 * 1024 * 1024:
                    raise ValueError("Template aliases require too much copying")
                shutil.copyfile(child, target)

    token = uuid.uuid4().hex
    key = "CODUX_PROJECT_CONTEXT_" + token
    captured = "codux_original_" + token
    wrapper = managed / ("codux-" + token + ".md")
    # Aliases may be hard links to the installation or the user's files. Never
    # truncate an alias, even if an original filename matches our generated one.
    with wrapper.open("x", encoding="utf-8") as output:
        output.write(
            "{% set " + captured + " %}{% include "
            + json.dumps(prompt.name, ensure_ascii=False) + " %}{% endset %}"
            + "${" + captured + ".rstrip()}\n\n${" + key + "}",
        )
    # JSON is a YAML subset. Inheritance retains tools, models, custom arguments
    # and subagent paths, with the latter still rooted at the original agent.
    generated = managed / ("codux-agent-" + token + ".yaml")
    with generated.open("x", encoding="utf-8") as output:
        json.dump({
            "version": 1,
            "agent": {
                "extend": str(base.resolve()),
                "system_prompt_path": str(wrapper),
                "system_prompt_args": {key: body},
            },
        }, output, ensure_ascii=False)
    # Validate inheritance using the same library the real CLI will execute.
    actual = load_agent_spec(generated)
    for name in ("name", "model", "tools", "allowed_tools", "exclude_tools", "subagents"):
        if getattr(actual, name, None) != getattr(spec, name, None):
            raise ValueError("Agent inheritance changed")
    return ["--agent-file", str(generated), *rest]


try:
    request_file = Path(sys.argv[1])
    try:
        request = json.loads(request_file.read_text(encoding="utf-8"))
    finally:
        request_file.unlink(missing_ok=True)
    result = prepare(request)
except Exception as error:
    result = {"error": "Unable to prepare Kimi project instructions (" + type(error).__name__ + ")"}
print(json.dumps(result, ensure_ascii=True))
