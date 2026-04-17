//! OSC sequence parser for extracting shell integration markers from the PTY
//! byte stream before feeding it to vt100.
//!
//! Handles OSC 133 (FinalTerm semantic prompts) and OSC 7 (CWD reporting).
//! Sequences may be split across multiple `feed()` calls.

/// Result of parsing an OSC sequence from the byte stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscResult {
    /// OSC 133 shell integration marker.
    Osc133(Osc133Marker),
    /// OSC 7 — working directory report (`file://host/path`).
    Osc7(String),
    /// OSC 0 or 2 — window title.
    Title(String),
}

/// OSC 133 FinalTerm semantic prompt markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Osc133Marker {
    /// `\e]133;A\a` — Prompt start.
    PromptStart,
    /// `\e]133;B\a` — Command input start (prompt rendered).
    CommandInputStart,
    /// `\e]133;C\a` — Command output start (user pressed Enter).
    CommandOutputStart,
    /// `\e]133;D;{exit_code}\a` — Command finished.
    CommandFinished { exit_code: Option<i32> },
}

/// State machine for scanning OSC sequences in a byte stream.
#[derive(Debug)]
pub struct OscParser {
    state: State,
    /// Accumulates the OSC body bytes (between `\e]` and the terminator).
    body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Normal passthrough.
    Normal,
    /// Saw `\x1b`, waiting for next byte.
    Esc,
    /// Saw `\x1b]`, accumulating OSC body.
    OscBody,
    /// Inside OSC body, saw `\x1b` — waiting for `\\` (ST terminator).
    OscEscSt,
}

impl OscParser {
    pub fn new() -> Self {
        Self {
            state: State::Normal,
            body: Vec::with_capacity(256),
        }
    }

    /// Feed a chunk of raw PTY output. Returns:
    /// - `results`: any extracted OSC results
    /// - `clean`: bytes to pass through to vt100 (OSC sequences stripped)
    pub fn feed(&mut self, data: &[u8]) -> (Vec<OscResult>, Vec<u8>) {
        let mut results = Vec::new();
        let mut clean = Vec::with_capacity(data.len());

        for &byte in data {
            match self.state {
                State::Normal => {
                    if byte == 0x1b {
                        self.state = State::Esc;
                    } else {
                        clean.push(byte);
                    }
                }
                State::Esc => {
                    if byte == b']' {
                        // Start of OSC sequence
                        self.state = State::OscBody;
                        self.body.clear();
                    } else {
                        // Not an OSC — pass ESC + this byte through
                        clean.push(0x1b);
                        clean.push(byte);
                        self.state = State::Normal;
                    }
                }
                State::OscBody => {
                    if byte == 0x07 {
                        // BEL terminator
                        if let Some(result) = self.parse_osc_body() {
                            results.push(result);
                        } else {
                            // Unknown OSC — pass it through for vt100
                            clean.push(0x1b);
                            clean.push(b']');
                            clean.extend_from_slice(&self.body);
                            clean.push(0x07);
                        }
                        self.body.clear();
                        self.state = State::Normal;
                    } else if byte == 0x1b {
                        // Might be ST (\x1b\\)
                        self.state = State::OscEscSt;
                    } else {
                        self.body.push(byte);
                    }
                }
                State::OscEscSt => {
                    if byte == b'\\' {
                        // ST terminator (\x1b\\)
                        if let Some(result) = self.parse_osc_body() {
                            results.push(result);
                        } else {
                            clean.push(0x1b);
                            clean.push(b']');
                            clean.extend_from_slice(&self.body);
                            clean.push(0x1b);
                            clean.push(b'\\');
                        }
                        self.body.clear();
                        self.state = State::Normal;
                    } else {
                        // False alarm — the ESC wasn't followed by \\
                        self.body.push(0x1b);
                        self.body.push(byte);
                        self.state = State::OscBody;
                    }
                }
            }
        }

        (results, clean)
    }

    /// Parse the accumulated OSC body. Returns `Some` for known sequences
    /// (133, 7, 0, 2), `None` for unknown ones.
    fn parse_osc_body(&self) -> Option<OscResult> {
        let body = std::str::from_utf8(&self.body).ok()?;

        if let Some(rest) = body.strip_prefix("133;") {
            return self.parse_osc133(rest);
        }
        if let Some(url) = body.strip_prefix("7;") {
            return Some(OscResult::Osc7(url.to_string()));
        }
        // OSC 0 (icon + title) or OSC 2 (title)
        if let Some(title) = body.strip_prefix("0;") {
            return Some(OscResult::Title(title.to_string()));
        }
        if let Some(title) = body.strip_prefix("2;") {
            return Some(OscResult::Title(title.to_string()));
        }

        None
    }

    fn parse_osc133(&self, params: &str) -> Option<OscResult> {
        let marker = match params.chars().next()? {
            'A' => Osc133Marker::PromptStart,
            'B' => Osc133Marker::CommandInputStart,
            'C' => Osc133Marker::CommandOutputStart,
            'D' => {
                // D may be followed by ;exit_code
                let exit_code = params
                    .strip_prefix("D;")
                    .and_then(|s| s.parse::<i32>().ok());
                Osc133Marker::CommandFinished { exit_code }
            }
            _ => return None,
        };
        Some(OscResult::Osc133(marker))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_osc133_prompt_start_bel() {
        let mut parser = OscParser::new();
        let input = b"\x1b]133;A\x07";
        let (results, clean) = parser.feed(input);
        assert_eq!(results, vec![OscResult::Osc133(Osc133Marker::PromptStart)]);
        assert!(clean.is_empty());
    }

    #[test]
    fn parse_osc133_command_finished_with_exit_code() {
        let mut parser = OscParser::new();
        let input = b"\x1b]133;D;0\x07";
        let (results, clean) = parser.feed(input);
        assert_eq!(
            results,
            vec![OscResult::Osc133(Osc133Marker::CommandFinished {
                exit_code: Some(0)
            })]
        );
        assert!(clean.is_empty());
    }

    #[test]
    fn parse_osc133_command_finished_no_exit_code() {
        let mut parser = OscParser::new();
        let input = b"\x1b]133;D\x07";
        let (results, clean) = parser.feed(input);
        assert_eq!(
            results,
            vec![OscResult::Osc133(Osc133Marker::CommandFinished {
                exit_code: None
            })]
        );
        assert!(clean.is_empty());
    }

    #[test]
    fn parse_osc7_cwd() {
        let mut parser = OscParser::new();
        let input = b"\x1b]7;file://localhost/home/user/project\x07";
        let (results, clean) = parser.feed(input);
        assert_eq!(
            results,
            vec![OscResult::Osc7(
                "file://localhost/home/user/project".to_string()
            )]
        );
        assert!(clean.is_empty());
    }

    #[test]
    fn parse_title_osc0() {
        let mut parser = OscParser::new();
        let input = b"\x1b]0;my terminal\x07";
        let (results, clean) = parser.feed(input);
        assert_eq!(results, vec![OscResult::Title("my terminal".to_string())]);
        assert!(clean.is_empty());
    }

    #[test]
    fn passthrough_normal_text() {
        let mut parser = OscParser::new();
        let input = b"hello world\r\n";
        let (results, clean) = parser.feed(input);
        assert!(results.is_empty());
        assert_eq!(clean, b"hello world\r\n");
    }

    #[test]
    fn mixed_osc_and_text() {
        let mut parser = OscParser::new();
        let input = b"before\x1b]133;A\x07after";
        let (results, clean) = parser.feed(input);
        assert_eq!(results, vec![OscResult::Osc133(Osc133Marker::PromptStart)]);
        assert_eq!(clean, b"beforeafter");
    }

    #[test]
    fn split_across_calls() {
        let mut parser = OscParser::new();

        // First chunk: ESC + start of OSC
        let (r1, c1) = parser.feed(b"text\x1b]13");
        assert!(r1.is_empty());
        assert_eq!(c1, b"text");

        // Second chunk: rest of OSC body + terminator
        let (r2, c2) = parser.feed(b"3;B\x07more");
        assert_eq!(r2, vec![OscResult::Osc133(Osc133Marker::CommandInputStart)]);
        assert_eq!(c2, b"more");
    }

    #[test]
    fn st_terminator() {
        let mut parser = OscParser::new();
        let input = b"\x1b]133;C\x1b\\";
        let (results, clean) = parser.feed(input);
        assert_eq!(
            results,
            vec![OscResult::Osc133(Osc133Marker::CommandOutputStart)]
        );
        assert!(clean.is_empty());
    }

    #[test]
    fn unknown_osc_passed_through() {
        let mut parser = OscParser::new();
        let input = b"\x1b]99;unknown\x07";
        let (results, clean) = parser.feed(input);
        assert!(results.is_empty());
        // Unknown OSC is passed through to vt100
        assert_eq!(clean, b"\x1b]99;unknown\x07");
    }

    #[test]
    fn non_osc_escape_passed_through() {
        let mut parser = OscParser::new();
        let input = b"\x1b[31m"; // CSI sequence, not OSC
        let (results, clean) = parser.feed(input);
        assert!(results.is_empty());
        assert_eq!(clean, b"\x1b[31m");
    }

    #[test]
    fn multiple_sequences_in_one_chunk() {
        let mut parser = OscParser::new();
        let input = b"\x1b]133;A\x07prompt$ \x1b]133;B\x07";
        let (results, clean) = parser.feed(input);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], OscResult::Osc133(Osc133Marker::PromptStart));
        assert_eq!(
            results[1],
            OscResult::Osc133(Osc133Marker::CommandInputStart)
        );
        assert_eq!(clean, b"prompt$ ");
    }

    #[test]
    fn all_four_osc133_markers() {
        let mut parser = OscParser::new();

        let (r, _) = parser.feed(b"\x1b]133;A\x07");
        assert_eq!(r[0], OscResult::Osc133(Osc133Marker::PromptStart));

        let (r, _) = parser.feed(b"\x1b]133;B\x07");
        assert_eq!(r[0], OscResult::Osc133(Osc133Marker::CommandInputStart));

        let (r, _) = parser.feed(b"\x1b]133;C\x07");
        assert_eq!(r[0], OscResult::Osc133(Osc133Marker::CommandOutputStart));

        let (r, _) = parser.feed(b"\x1b]133;D;127\x07");
        assert_eq!(
            r[0],
            OscResult::Osc133(Osc133Marker::CommandFinished {
                exit_code: Some(127)
            })
        );
    }
}
