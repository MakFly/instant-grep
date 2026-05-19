"""ig-rewrite plugin for Hermes.

Installed by `ig setup --agent hermes` to ~/.hermes/plugins/ig-rewrite/.
Rewrites shell-command requests from `grep`/`rg`/`find`/`cat` to their
trigram-indexed `ig` equivalents before execution.

This is a best-effort shim; if the host Hermes version lacks the plugin
hook surface we expect, the file is harmless dead code.
"""

PLUGIN_NAME = "ig-rewrite"
PLUGIN_VERSION = "1.0.0"


def rewrite(command: str) -> str:
    """Return an ig-equivalent command, or the original if no rewrite applies."""
    stripped = command.strip()
    if stripped.startswith("grep ") or stripped.startswith("rg "):
        return "ig " + stripped.split(" ", 1)[1]
    if stripped.startswith("find ") and " -name " in stripped:
        return "ig files"
    if stripped.startswith("cat "):
        return "ig read " + stripped[4:]
    return command


def register(hermes):  # pragma: no cover - hook signature varies by host
    try:
        hermes.pre_exec(rewrite)
    except AttributeError:
        pass
