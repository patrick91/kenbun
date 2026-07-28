//! Conservative shell-command recognition for package scripts.

pub(super) fn command_invokes(script: &str, command: &str) -> bool {
    command_occurrences(script, command).next().is_some()
}

pub(super) fn command_invokes_subcommand(script: &str, command: &str, subcommand: &str) -> bool {
    command_occurrences(script, command).any(|(tokens, command_index)| {
        let mut index = command_index + 1;
        while index < tokens.len() && !is_shell_operator(&tokens[index]) {
            let token = &tokens[index];
            if token == "--" {
                index += 1;
                return tokens.get(index).is_some_and(|token| token == subcommand);
            }
            if token.starts_with('-') {
                if option_takes_value(command, token) && !token.contains('=') {
                    index += 1;
                }
                index += 1;
                continue;
            }
            return token == subcommand;
        }
        false
    })
}

fn option_takes_value(command: &str, option: &str) -> bool {
    match command {
        "vite" => matches!(
            option,
            "--config" | "-c" | "--base" | "--mode" | "-m" | "--logLevel" | "--host" | "--port"
        ),
        "react-router" => matches!(option, "--config" | "-c" | "--mode" | "-m"),
        _ => false,
    }
}

fn command_occurrences<'a>(
    script: &'a str,
    command: &'a str,
) -> impl Iterator<Item = (Vec<String>, usize)> + 'a {
    split_shell_segments(script)
        .into_iter()
        .filter_map(move |segment| {
            let tokens = shell_words(&segment);
            recognized_command_index(&tokens, command).map(|index| (tokens, index))
        })
}

fn recognized_command_index(tokens: &[String], command: &str) -> Option<usize> {
    let mut index = 0;
    while index < tokens.len() && is_environment_assignment(&tokens[index]) {
        index += 1;
    }
    let first_executable = tokens.get(index).map(|token| executable_name(token));
    if matches!(
        first_executable.as_deref(),
        Some("cross-env" | "cross-env-shell")
    ) {
        index += 1;
        while index < tokens.len()
            && (tokens[index].starts_with('-') || is_environment_assignment(&tokens[index]))
        {
            index += 1;
        }
    }
    let executable = tokens.get(index).map(|token| executable_name(token))?;
    if executable == command {
        return Some(index);
    }

    match executable.as_str() {
        "npx" | "bunx" => find_wrapped_command(tokens, index + 1, command, &[]),
        "pnpm" | "yarn" => find_wrapped_command(tokens, index + 1, command, &["exec", "dlx", "x"]),
        "npm" => {
            let tail = &tokens[index + 1..];
            if tail
                .iter()
                .any(|token| matches!(token.as_str(), "exec" | "x"))
            {
                find_wrapped_command(tokens, index + 1, command, &["exec", "x"])
            } else {
                None
            }
        }
        "bun" => find_wrapped_command(tokens, index + 1, command, &["x"]),
        "node" => tokens
            .iter()
            .enumerate()
            .skip(index + 1)
            .find_map(|(index, token)| {
                let normalized = token.replace('\\', "/");
                (normalized.contains(&format!("/{command}/"))
                    || normalized.ends_with(&format!("/{command}")))
                .then_some(index)
            }),
        _ => None,
    }
}

fn find_wrapped_command(
    tokens: &[String],
    start: usize,
    command: &str,
    ignorable_words: &[&str],
) -> Option<usize> {
    tokens
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, token)| {
            let executable = executable_name(token);
            if token.starts_with('-')
                || token == "--"
                || ignorable_words.contains(&executable.as_str())
                || is_environment_assignment(token)
            {
                None
            } else if executable == command {
                Some(index)
            } else {
                // A different executable means a wrapper such as `npm run foo`;
                // do not treat later mentions as command execution.
                Some(usize::MAX)
            }
        })
        .filter(|index| *index != usize::MAX)
}

fn split_shell_segments(script: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for character in script.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if !single => escaped = true,
            '\'' if !double => {
                single = !single;
                current.push(character);
            }
            '"' if !single => {
                double = !double;
                current.push(character);
            }
            ';' | '&' | '|' if !single && !double => {
                if !current.trim().is_empty() {
                    segments.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }
    segments
}

fn shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for character in segment.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            character if character.is_whitespace() && !single && !double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn executable_name(token: &str) -> String {
    let name = token
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(token)
        .to_ascii_lowercase();
    for suffix in [".cmd", ".exe", ".ps1"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    name
}

fn is_environment_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'))
}

fn is_shell_operator(token: &str) -> bool {
    matches!(token, ";" | "&" | "&&" | "|" | "||")
}
