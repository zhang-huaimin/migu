/// Generate the shell initialization snippet for the given shell.
pub fn init_script(shell: &str) -> &'static str {
    match shell {
        "bash" => BASH_INIT,
        "zsh" => ZSH_INIT,
        "fish" => FISH_INIT,
        _ => unreachable!(),
    }
}

const BASH_INIT: &str = r###"
# --- migu: cross-shell command history ---
# Add this to ~/.bashrc:
#   eval "$(migu init bash)"

# Record every command before the prompt is displayed
_migu_prompt_command() {
    local cmd
    cmd="$(history 1 | sed 's/^ *[0-9][0-9]* *[0-9]\{4\}-[0-9][0-9]-[0-9][0-9] [0-9][0-9]:[0-9][0-9]:[0-9][0-9] *//' | sed 's/^ *[0-9][0-9]* *//')"
    migu add -- "$cmd"
}
PROMPT_COMMAND=_migu_prompt_command

# Ctrl-R widget: two-keystroke macro (like mcfly).
#   Ctrl-R → MIGU_SEARCH_KEY + MIGU_ACCEPT_KEY.
#   MIGU_SEARCH_KEY runs _migu_widget via bind -x.
#   MIGU_ACCEPT_KEY is dynamically bound to accept-line (Enter) or nothing (Tab/cancel).
MIGU_SEARCH_KEY="${MIGU_SEARCH_KEY:-\C-x1}"
MIGU_ACCEPT_KEY="${MIGU_ACCEPT_KEY:-\C-x2}"

_migu_widget() {
    MIGU_WIDGET=1 command migu
    local cmd
    cmd="$(cat /tmp/migu-cmd 2>/dev/null)"
    if [ -n "$cmd" ]; then
        if [ -f /tmp/migu-exec ]; then
            # Enter: insert + auto-execute
            rm -f /tmp/migu-exec /tmp/migu-cmd
            READLINE_LINE="$cmd"
            READLINE_POINT=${#READLINE_LINE}
            bind -m emacs     "\"${MIGU_ACCEPT_KEY}\":accept-line"
            bind -m vi-insert "\"${MIGU_ACCEPT_KEY}\":accept-line"
        else
            # Tab: insert only (editable)
            rm -f /tmp/migu-cmd
            READLINE_LINE="$cmd"
            READLINE_POINT=${#READLINE_LINE}
            bind -m emacs     "\"${MIGU_ACCEPT_KEY}\":\"\""
            bind -m vi-insert "\"${MIGU_ACCEPT_KEY}\":\"\""
        fi
    else
        # Cancelled: ensure accept key does nothing
        bind -m emacs     "\"${MIGU_ACCEPT_KEY}\":\"\""
        bind -m vi-insert "\"${MIGU_ACCEPT_KEY}\":\"\""
    fi
}

bind -m emacs     -x "\"${MIGU_SEARCH_KEY}\":_migu_widget" 2>/dev/null
bind -m vi-insert -x "\"${MIGU_SEARCH_KEY}\":_migu_widget" 2>/dev/null
bind -m emacs     "\"\C-r\":\"${MIGU_SEARCH_KEY}${MIGU_ACCEPT_KEY}\""
bind -m vi-insert "\"\C-r\":\"${MIGU_SEARCH_KEY}${MIGU_ACCEPT_KEY}\""

# Import existing history on first setup (runs in background)
(migu import bash 2>/dev/null &)
"###;

const ZSH_INIT: &str = r###"
# --- migu: cross-shell command history ---
# Add this to ~/.zshrc:
#   eval "$(migu init zsh)"

autoload -Uz add-zsh-hook

_migu_add_hook() {
    # recursion guard: preexec fires for migu add itself
    [[ -n "$_migu_skip" ]] && return
    _migu_skip=1
    migu add -- "$1"
    unset _migu_skip
}
add-zsh-hook preexec _migu_add_hook

# Ctrl-R widget
_migu_widget() {
    MIGU_WIDGET=1 command migu
    local cmd="$(cat /tmp/migu-cmd 2>/dev/null)"
    if [ -n "$cmd" ]; then
        if [ -f /tmp/migu-exec ]; then
            # Enter: insert + auto-execute
            rm -f /tmp/migu-exec
            rm -f /tmp/migu-cmd
            zle reset-prompt
            LBUFFER+="$cmd"
            zle accept-line
        else
            # Tab: insert only (editable)
            rm -f /tmp/migu-cmd
            zle reset-prompt
            LBUFFER+="$cmd"
        fi
    fi
}
zle -N _migu_widget
bindkey '^R' _migu_widget

# Import existing history on first setup (runs in background)
(migu import zsh 2>/dev/null &)
"###;

const FISH_INIT: &str = r###"
# --- migu: cross-shell command history ---
# Add this to ~/.config/fish/config.fish:
#   migu init fish | source

function _migu_add --on-event fish_preexec
    # recursion guard
    if set -q _migu_skip
        return
    end
    set -g _migu_skip 1
    migu add -- "$argv"
    set -e _migu_skip
end

# Ctrl-R widget
function _migu_widget
    MIGU_WIDGET=1 command migu
    set -l cmd (cat /tmp/migu-cmd 2>/dev/null)
    if test -n "$cmd"
        if test -f /tmp/migu-exec
            # Enter: insert + auto-execute
            rm -f /tmp/migu-exec
            rm -f /tmp/migu-cmd
            commandline -r -- $cmd
            commandline -f execute
        else
            # Tab: insert only (editable)
            rm -f /tmp/migu-cmd
            commandline -r -- $cmd
        end
    end
end
bind \cr _migu_widget

# Import existing history on first setup (runs in background)
migu import fish 2>/dev/null &
"###;
