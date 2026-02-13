use rust_mcp_schema::PromptArgument;
use std::collections::HashMap;

pub(super) fn parse_prompt_args(
    input: &str,
    prompt_args: &[PromptArgument],
) -> Result<HashMap<String, String>, String> {
    if input.trim().is_empty() {
        return Ok(HashMap::new());
    }

    if prompt_args.len() == 1 {
        match parse_kv_args(input) {
            Ok(map) => return Ok(map),
            Err(_) => {
                let value = parse_single_prompt_value(input)?;
                let mut args = HashMap::new();
                args.insert(prompt_args[0].name.clone(), value);
                return Ok(args);
            }
        }
    }

    parse_kv_args(input)
}

pub(super) fn validate_prompt_args(
    args: &HashMap<String, String>,
    prompt_args: &[PromptArgument],
) -> Result<(), String> {
    let mut allowed: Vec<&str> = prompt_args.iter().map(|arg| arg.name.as_str()).collect();

    for key in args.keys() {
        if !allowed.iter().any(|name| name == key) {
            allowed.sort();
            let allowed_list = if allowed.is_empty() {
                "none".to_string()
            } else {
                allowed.join(", ")
            };
            return Err(format!(
                "Unknown prompt argument '{}'. Allowed: {}.",
                key, allowed_list
            ));
        }
    }

    Ok(())
}

pub(super) fn parse_kv_args(input: &str) -> Result<HashMap<String, String>, String> {
    if input.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let tokens = tokenize_prompt_args(input)?;
    let mut args = HashMap::new();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!(
                "Invalid prompt argument '{}'. Use key=value.",
                token
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err("Prompt argument name cannot be empty.".to_string());
        }
        args.insert(key.to_string(), value.to_string());
    }

    Ok(args)
}

fn parse_single_prompt_value(input: &str) -> Result<String, String> {
    let tokens = tokenize_prompt_args(input)?;
    if tokens.is_empty() {
        return Ok(String::new());
    }
    if tokens.len() == 1 {
        return Ok(tokens[0].clone());
    }
    Ok(tokens.join(" "))
}

fn tokenize_prompt_args(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for ch in input.chars() {
        match ch {
            '"' | '\'' => {
                if let Some(q) = in_quote {
                    if q == ch {
                        in_quote = None;
                    } else {
                        current.push(ch);
                    }
                } else {
                    in_quote = Some(ch);
                }
            }
            c if c.is_whitespace() && in_quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if let Some(q) = in_quote {
        return Err(format!("Unclosed quote ({}) in prompt arguments.", q));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_driven_prompt_parsing() {
        let one_arg = vec![PromptArgument {
            name: "topic".to_string(),
            title: None,
            description: None,
            required: Some(true),
        }];
        let many_args = vec![
            PromptArgument {
                name: "topic".to_string(),
                title: None,
                description: None,
                required: Some(true),
            },
            PromptArgument {
                name: "lang".to_string(),
                title: None,
                description: None,
                required: Some(false),
            },
        ];

        struct Case<'a> {
            input: &'a str,
            schema: &'a [PromptArgument],
            expected: Result<Vec<(&'a str, &'a str)>, &'a str>,
        }

        let cases = vec![
            Case {
                input: "topic=soil lang=en",
                schema: &many_args,
                expected: Ok(vec![("topic", "soil"), ("lang", "en")]),
            },
            Case {
                input: "topic='soil health'",
                schema: &many_args,
                expected: Ok(vec![("topic", "soil health")]),
            },
            Case {
                input: "soil health",
                schema: &one_arg,
                expected: Ok(vec![("topic", "soil health")]),
            },
            Case {
                input: "topic",
                schema: &many_args,
                expected: Err("Invalid prompt argument 'topic'. Use key=value."),
            },
            Case {
                input: "topic='open",
                schema: &many_args,
                expected: Err("Unclosed quote (') in prompt arguments."),
            },
        ];

        for case in cases {
            let parsed = parse_prompt_args(case.input, case.schema);
            match (parsed, case.expected) {
                (Ok(map), Ok(pairs)) => {
                    for (key, value) in pairs {
                        assert_eq!(map.get(key).map(String::as_str), Some(value));
                    }
                }
                (Err(err), Err(expected)) => assert_eq!(err, expected),
                (outcome, expected) => {
                    panic!("unexpected parse result: {:?} vs {:?}", outcome, expected)
                }
            }
        }
    }
}
