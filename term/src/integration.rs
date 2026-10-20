//! Shell integration: small scripts that make a shell mark its prompts and command output (OSC 133),
//! and the installer that adds them to the shell's startup file.

use std::path::PathBuf;

use crate::config::config_dir;

const BASH: &str = r#"# term shell integration for bash: marks prompts and command output (OSC 133).
[[ $- == *i* ]] || return 0
[[ -n "${__term_integration:-}" ]] && return 0
__term_integration=1
__term_first=1
__term_prompt() {
    local status=$?
    if [[ -z $__term_first ]]; then printf '\e]133;D;%s\a' "$status"; fi
    __term_first=
    printf '\e]133;A\a'
    return $status
}
if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(__term_prompt "${PROMPT_COMMAND[@]}")
else
    PROMPT_COMMAND="__term_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
PS1="${PS1}\[\e]133;B\a\]"
PS0='\e]133;C\a'"${PS0:-}"
"#;

const ZSH: &str = r#"# term shell integration for zsh: marks prompts and command output (OSC 133).
[[ -o interactive ]] || return 0
(( ${+__term_integration} )) && return 0
typeset -g __term_integration=1 __term_ran=
autoload -Uz add-zsh-hook
__term_precmd() {
    local s=$?
    [[ -n $__term_ran ]] && print -n -- "\e]133;D;$s\a"
    __term_ran=
    print -n -- "\e]133;A\a"
}
__term_precmd_last() {
    [[ $PS1 == *'133;B'* ]] || PS1="${PS1}%{"$'\e]133;B\a'"%}"
}
__term_preexec() {
    __term_ran=1
    print -n -- "\e]133;C\a"
}
add-zsh-hook precmd __term_precmd
add-zsh-hook precmd __term_precmd_last
add-zsh-hook preexec __term_preexec
# Run first, so $? is still the command's status.
precmd_functions=(__term_precmd ${precmd_functions:#__term_precmd})
"#;

const FISH: &str = r#"# term shell integration for fish: marks prompts and command output (OSC 133).
status is-interactive; or exit
set -q __term_integration; and exit
set -g __term_integration 1
function __term_preexec --on-event fish_preexec
    printf '\e]133;C\a'
end
function __term_postexec --on-event fish_postexec
    set -l s $status
    printf '\e]133;D;%s\a' $s
end
functions -q fish_prompt; and functions -c fish_prompt __term_original_prompt
function fish_prompt
    printf '\e]133;A\a'
    __term_original_prompt
    printf '\e]133;B\a'
end
"#;

const POWERSHELL: &str = r#"# term shell integration for PowerShell: marks prompts and command output (OSC 133).
if ($global:__term_integration) { return }
$global:__term_integration = $true
$global:__term_original_prompt = $function:prompt
function global:prompt {
    $ok = $?
    $esc = [char]27; $bel = [char]7
    $code = if ($ok) { 0 } elseif ($LASTEXITCODE) { $LASTEXITCODE } else { 1 }
    $out = ''
    if ($global:__term_ran) { $out += "$esc]133;D;$code$bel" }
    $global:__term_ran = $false
    $out += "$esc]133;A$bel"
    $out += & $global:__term_original_prompt
    $out += "$esc]133;B$bel"
    $out
}
if (Get-Module PSReadLine) {
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        $global:__term_ran = $true
        [Console]::Write("$([char]27)]133;C$([char]7)")
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}
"#;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl Shell {
    pub const ALL: [Shell; 4] = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell];

    pub fn name(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::PowerShell => "PowerShell",
        }
    }

    fn script_name(self) -> &'static str {
        match self {
            Shell::Bash => "term.bash",
            Shell::Zsh => "term.zsh",
            Shell::Fish => "term.fish",
            Shell::PowerShell => "term.ps1",
        }
    }

    fn script(self) -> &'static str {
        match self {
            Shell::Bash => BASH,
            Shell::Zsh => ZSH,
            Shell::Fish => FISH,
            Shell::PowerShell => POWERSHELL,
        }
    }

    /// The startup file the integration is hooked into.
    pub fn rc_file(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(match self {
            Shell::Bash => home.join(".bashrc"),
            Shell::Zsh => std::env::var_os("ZDOTDIR").map(PathBuf::from).filter(|d| d.is_dir()).unwrap_or(home).join(".zshrc"),
            Shell::Fish => dirs::config_dir()?.join("fish").join("conf.d").join("term.fish"),
            Shell::PowerShell => {
                if cfg!(windows) {
                    dirs::document_dir()?.join("PowerShell").join("Microsoft.PowerShell_profile.ps1")
                } else {
                    dirs::config_dir()?.join("powershell").join("Microsoft.PowerShell_profile.ps1")
                }
            }
        })
    }

    /// The lines added to the startup file.
    pub fn snippet(self) -> String {
        let hook = match self {
            Shell::Bash | Shell::Zsh => {
                let f = self.script_name();
                format!("[ \"$TERM_PROGRAM\" = term ] && [ -r \"$TERM_INTEGRATION_DIR/{f}\" ] && . \"$TERM_INTEGRATION_DIR/{f}\"")
            }
            Shell::Fish => {
                "if test \"$TERM_PROGRAM\" = term; and test -r \"$TERM_INTEGRATION_DIR/term.fish\"; source \"$TERM_INTEGRATION_DIR/term.fish\"; end"
                    .to_string()
            }
            Shell::PowerShell => {
                "if ($env:TERM_PROGRAM -eq 'term' -and (Test-Path \"$env:TERM_INTEGRATION_DIR/term.ps1\")) { . \"$env:TERM_INTEGRATION_DIR/term.ps1\" }"
                    .to_string()
            }
        };
        format!("{START}\n{hook}\n{END}\n")
    }

    pub fn installed(self) -> bool {
        self.rc_file().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| t.contains(START))
    }

    /// Write the scripts and hook them into the shell's startup file. Returns that file's path.
    pub fn install(self) -> Result<PathBuf, String> {
        ensure()?;
        let rc = self.rc_file().ok_or("could not find your home folder")?;
        let old = std::fs::read_to_string(&rc).unwrap_or_default();
        if old.contains(START) {
            return Ok(rc);
        }
        if let Some(dir) = rc.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let mut text = old;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&self.snippet());
        std::fs::write(&rc, text).map_err(|e| format!("could not write {}: {e}", rc.display()))?;
        Ok(rc)
    }

    /// The shell named by `$SHELL` (or PowerShell on Windows).
    pub fn detect() -> Option<Shell> {
        if cfg!(windows) {
            return Some(Shell::PowerShell);
        }
        let shell = std::env::var("SHELL").ok()?;
        let name = shell.rsplit('/').next()?;
        Shell::ALL.into_iter().find(|s| name == s.name().to_lowercase() || (name == "pwsh" && *s == Shell::PowerShell))
    }
}

const START: &str = "# >>> term shell integration >>>";
const END: &str = "# <<< term shell integration <<<";

pub fn dir() -> String {
    config_dir().join("shell-integration").display().to_string()
}

/// Make sure the scripts exist and are current.
pub fn ensure() -> Result<(), String> {
    let dir = PathBuf::from(dir());
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    for shell in Shell::ALL {
        let path = dir.join(shell.script_name());
        if std::fs::read_to_string(&path).ok().as_deref() != Some(shell.script()) {
            std::fs::write(&path, shell.script()).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_are_guarded_and_marked() {
        for s in Shell::ALL {
            let snip = s.snippet();
            assert!(snip.starts_with(START) && snip.trim_end().ends_with(END), "{snip}");
            assert!(snip.contains("TERM_INTEGRATION_DIR"));
            assert!(s.script().contains("133;A"));
        }
    }
}
