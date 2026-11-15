/// What the binary was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Collect,
    Login,
}

impl Command {
    /// Reads the arguments after the program name; `None` means a usage error.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Option<Self> {
        let args: Vec<String> = args.into_iter().collect();
        match args.as_slice() {
            [] => Some(Self::Collect),
            [word] if word == "login" => Some(Self::Login),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Command;

    fn parse(args: &[&str]) -> Option<Command> {
        Command::parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn no_argument_collects() {
        assert_eq!(parse(&[]), Some(Command::Collect));
    }

    #[test]
    fn login_logs_in() {
        assert_eq!(parse(&["login"]), Some(Command::Login));
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        assert_eq!(parse(&["bogus"]), None);
        assert_eq!(parse(&["login", "extra"]), None);
    }
}
