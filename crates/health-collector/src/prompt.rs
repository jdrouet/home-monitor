use std::io::{self, BufRead, Write};

/// Writes `label`, then reads one line without its line ending.
pub fn prompt_line(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> io::Result<String> {
    write!(output, "{label}: ")?;
    output.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::prompt_line;

    #[test]
    fn prints_the_label_and_returns_the_line() {
        let mut input = "123456\r\nnext\n".as_bytes();
        let mut output = Vec::new();
        let line = prompt_line(&mut input, &mut output, "MFA code").expect("line");
        assert_eq!(line, "123456");
        assert_eq!(output, b"MFA code: ");
    }

    #[test]
    fn closed_input_is_an_error() {
        let mut input = "".as_bytes();
        let error = prompt_line(&mut input, &mut Vec::new(), "Email").expect_err("eof");
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    }
}
