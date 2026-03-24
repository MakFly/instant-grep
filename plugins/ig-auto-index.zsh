# ig auto-index — source this file from .zshrc
# Automatically indexes git projects when you cd into them.
#
# Usage: add to ~/.zshrc:
#   source ~/Documents/lab/sandbox/instant-grep/plugins/ig-auto-index.zsh

_ig_auto_index() {
    # Only trigger in git repos
    [[ -d .git ]] || return

    local ig_bin="${HOME}/.local/bin/ig"
    [[ -x "$ig_bin" ]] || return

    # Skip if index is fresh (modified <5 min ago)
    local meta=".ig/metadata.bin"
    if [[ -f "$meta" ]]; then
        local age=$(( $(date +%s) - $(stat -f %m "$meta" 2>/dev/null || echo 0) ))
        (( age < 300 )) && return
    fi

    # Background index (silent, won't block shell)
    "$ig_bin" index . &>/dev/null &
    disown
}

# Hook into zsh directory change
autoload -Uz add-zsh-hook
add-zsh-hook chpwd _ig_auto_index

# Also run on shell startup (for the initial directory)
_ig_auto_index
