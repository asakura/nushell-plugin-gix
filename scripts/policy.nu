# Code-policy checks that the compiler and clippy cannot express; run by gate and CI. Prints every
# violation and exits 1 when there is any; otherwise prints `policy: green`.

# Runs every check from the project root (the same root gate uses) and reports all violations.
def main [] {
    cd ($env.CLAUDE_PROJECT_DIR? | default --empty (^git rev-parse --show-toplevel | str trim))
    let violations = [ (core-imports-nushell) ] | flatten
    if ($violations | is-not-empty) {
        for violation in $violations { print --stderr $violation }
        exit 1
    }
    print --stderr 'policy: green'
}

# nu-plugin-gix-core is the Nushell-free git side: its sources name no nu_protocol or nu_plugin
# path, and its manifest has no nu-plugin or nu-protocol dependency key. The key pattern requires
# `.` or `=` after the name, so nu-plugin-gix-testing-support (a dev-dependency) is not matched.
def core-imports-nushell []: nothing -> list<string> {
    let rule = '.claude/rules/core.md [core] must_not'
    let grep = ^git grep -nE '\bnu_(protocol|plugin)(\b|::)' -- 'nu-plugin-gix-core/*.rs' | complete
    let sources = match $grep.exit_code {
        0 => {
            $grep.stdout | lines | each {|hit|
                let parts = $hit | split row ':'
                let file = $parts | first
                let line = $parts | get 1
                $"($file):($line): source names a nu_protocol or nu_plugin path; core must not depend on Nushell \(($rule)\)"
            }
        }
        1 => { [] }
        _ => { error make {msg: $"git grep failed: ($grep.stderr)"} }
    }
    let manifest = 'nu-plugin-gix-core/Cargo.toml'
    let keys = open --raw $manifest | lines | enumerate
        | where {|row| $row.item =~ '^nu-(plugin|protocol)\s*[.=]' }
        | each {|row|
            let line = $row.index + 1
            $"($manifest):($line): depends on a Nushell crate; core must not depend on nu-plugin or nu-protocol \(($rule)\)"
        }
    $sources ++ $keys
}
