use std::iter::Peekable;
use std::ops::Range;
use std::path::PathBuf;

use regex::Regex;
use unicode_segmentation::Graphemes;
use unicode_segmentation::UnicodeSegmentation;

use line_edit::EditorOptions;
use line_edit::LineEdit;

use crate::edit_buffer::EditBuffer;
use crate::eol::{Eol, IsEol};
use crate::error::Error;
use crate::iter_utils::Peeking;

#[derive(Debug)]
pub enum Cmd {
    Append {
        index: Option<usize>,
        source: InputSource,
        mode: InputMode,
    },
    Copy(Option<Range<usize>>),
    Cut(Option<Range<usize>>),
    Delete(Option<Range<usize>>),
    Edit(PathBuf),
    Enumerate(Option<Range<usize>>),
    File,
    Global(Option<Range<usize>>, Regex, String),
    Insert {
        index: Option<usize>,
        source: InputSource,
        mode: InputMode,
    },
    Join(Option<Range<usize>>, Option<String>),
    Justify {
        span: Option<Range<usize>>,
        wrap: Wrapping,
        left_margin: Option<usize>,
        line_width: Option<usize>,
    },
    LineNumber(Option<usize>),
    List(Option<Range<usize>>),
    Newline(Option<Eol>),
    New,
    Null(Option<usize>),
    Overwrite {
        span: Option<Range<usize>>,
        source: InputSource,
        mode: InputMode,
    },
    PageDown(Option<usize>, Option<usize>, Option<PrintSuffix>),
    PageUp(Option<usize>, Option<usize>, Option<PrintSuffix>),
    Print(Option<Range<usize>>),
    Reload,
    Quit,
    Redo,
    ShowDiff(Option<PathBuf>),
    Substitute(Option<Range<usize>>, Regex, String, SubstitutionScope),
    Undo,
    Version,
    Write,
    WriteAs(Option<Range<usize>>, PathBuf),
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum SubstitutionScope {
    Single(usize),
    Global,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PrintSuffix {
    pub enumerate: bool,
    pub expand_escapes: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum InputMode {
    Cooked,
    Raw,
}

#[derive(Debug, PartialEq)]
pub enum InputSource {
    Clipboard,
    File(PathBuf),
    StdIn,
}

#[derive(Debug, Default, PartialEq)]
pub enum Wrapping {
    #[default]
    Fill,
    NoFill,
    None,
}

impl Cmd {
    /// Read input, parsing into a Cmd
    pub fn read(
        input: &mut impl LineEdit,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
        let cmd_input_options = EditorOptions {
            prompt: Some(':'),
            history: true,
            ..Default::default()
        };
        let mut line = String::with_capacity(120);
        input
            .read_line(&mut line, Some(&cmd_input_options))
            .map_err(|e| Error::ReadCommand { source: Some(Box::new(e)) })?;
        if line.is_empty() {
            return Ok(None);
        }
        let mut graphemes = line.as_str().graphemes(true).peekable();
        let address = eval_address(&mut graphemes, buffer, previous_pattern)?;
        match graphemes.next() {
            Some("a") => {
                parse_append_cmd(&mut graphemes, address, InputMode::Cooked)
            }
            Some("A") => {
                parse_append_cmd(&mut graphemes, address, InputMode::Raw)
            }
            Some("c") => parse_no_args(&mut graphemes, Cmd::Copy(address)),
            Some("d") => parse_no_args(&mut graphemes, Cmd::Delete(address)),
            Some("e") => parse_edit_cmd(&mut graphemes, address.as_ref()),
            Some("E") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::Reload)
            }
            Some("f") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::File)
            }
            Some("g") => parse_global_cmd(
                &mut graphemes,
                input,
                previous_pattern,
                address,
            ),
            Some("i") => {
                parse_insert_cmd(&mut graphemes, address, InputMode::Cooked)
            }
            Some("I") => {
                parse_insert_cmd(&mut graphemes, address, InputMode::Raw)
            }
            Some("j") => parse_join_cmd(&mut graphemes, address),
            Some("J") => parse_justify_cmd(&mut graphemes, address),
            Some("l") => parse_no_args(&mut graphemes, Cmd::List(address)),
            Some("L") => parse_newline_cmd(&mut graphemes, address.as_ref()),
            Some("n") => parse_no_args(&mut graphemes, Cmd::Enumerate(address)),
            Some("N") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::New)
            }
            None | Some("\n" | "\r\n") => {
                Ok(Some((Cmd::Null(address.map(|a| a.end - 1)), None)))
            }
            Some("o") => {
                parse_overwrite_cmd(&mut graphemes, address, InputMode::Cooked)
            }
            Some("O") => {
                parse_overwrite_cmd(&mut graphemes, address, InputMode::Raw)
            }
            Some("p") => parse_no_args(&mut graphemes, Cmd::Print(address)),
            Some("q") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::Quit)
            }
            Some("S") => parse_show_cmd(&mut graphemes, address.as_ref()),
            Some("s") => parse_substitute_cmd(
                &mut graphemes,
                input,
                buffer,
                previous_pattern,
                address,
            ),
            Some("u") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::Undo)
            }
            Some("#") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::Version)
            }
            Some("U") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::Redo)
            }
            Some("w") => {
                parse_simple_cmd(&mut graphemes, address.as_ref(), Cmd::Write)
            }
            Some("W") => parse_write_as_cmd(&mut graphemes, address),
            Some("x") => parse_no_args(&mut graphemes, Cmd::Cut(address)),
            Some("z") => {
                parse_page_down_cmd(&mut graphemes, address.map(|a| a.end - 1))
            }
            Some("Z") => {
                parse_page_up_cmd(&mut graphemes, address.map(|a| a.end - 1))
            }
            Some("=") => parse_no_args(
                &mut graphemes,
                Cmd::LineNumber(address.map(|a| a.end - 1)),
            ),
            Some(s) => Err(Error::UnknownCmd(s.to_owned())),
        }
    }
}

fn parse_print_suffix(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<Option<PrintSuffix>, Error> {
    let mut pr_sfx: Option<PrintSuffix> = None;
    loop {
        let gr = graphemes.next();
        match gr {
            None | Some("\n" | "\r\n") => break,
            Some("n") => pr_sfx.get_or_insert_default().enumerate = true,
            Some("p") => {
                pr_sfx.get_or_insert_default();
            }
            Some("l") => pr_sfx.get_or_insert_default().expand_escapes = true,
            _ => return Err(Error::InvalidCmdSuffix),
        }
    }
    Ok(pr_sfx)
}

fn parse_append_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<Range<usize>>,
    mode: InputMode,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let index = address.map(|a| a.end - 1);
    let source = match graphemes.peek() {
        Some(&"v") => {
            graphemes.next();
            InputSource::Clipboard
        }
        Some(&" " | &"\t") => {
            graphemes.next();
            let mut filename = graphemes
                .take_while(|s| Eol::from_line(s).is_none())
                .collect::<String>();
            filename.retain(|c| !c.is_whitespace());
            if filename.is_empty() {
                InputSource::StdIn
            } else {
                InputSource::File(PathBuf::from(filename))
            }
        }
        _ => InputSource::StdIn,
    };

    let pr_sfx = parse_print_suffix(graphemes)?;
    Ok(Some((Cmd::Append { index, source, mode }, pr_sfx)))
}

fn parse_insert_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<Range<usize>>,
    mode: InputMode,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let index = address.map(|a| a.end - 1);
    let source = match graphemes.peek() {
        Some(&"v") => {
            graphemes.next();
            InputSource::Clipboard
        }
        Some(&" " | &"\t") => {
            graphemes.next();
            let mut filename = graphemes
                .take_while(|s| Eol::from_line(s).is_none())
                .collect::<String>();
            filename.retain(|c| !c.is_whitespace());
            if filename.is_empty() {
                InputSource::StdIn
            } else {
                InputSource::File(PathBuf::from(filename))
            }
        }
        _ => InputSource::StdIn,
    };

    let pr_sfx = parse_print_suffix(graphemes)?;
    Ok(Some((Cmd::Insert { index, source, mode }, pr_sfx)))
}

fn parse_overwrite_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    span: Option<Range<usize>>,
    mode: InputMode,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let source = match graphemes.peek() {
        Some(&"v") => {
            graphemes.next();
            InputSource::Clipboard
        }
        Some(&" " | &"\t") => {
            graphemes.next();
            let mut filename = graphemes
                .take_while(|s| Eol::from_line(s).is_none())
                .collect::<String>();
            filename.retain(|c| !c.is_whitespace());
            if filename.is_empty() {
                InputSource::StdIn
            } else {
                InputSource::File(PathBuf::from(filename))
            }
        }
        _ => InputSource::StdIn,
    };

    let pr_sfx = parse_print_suffix(graphemes)?;
    Ok(Some((Cmd::Overwrite { span, source, mode }, pr_sfx)))
}

fn parse_write_as_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Err(Error::NoFilename),
        Some(" " | "\t") => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::NoFilename)
            } else {
                Ok(Some((Cmd::WriteAs(address, PathBuf::from(filename)), None)))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_page_down_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    index: Option<usize>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let window = parse_usize(graphemes)?;
    let print_sfx = parse_print_suffix(graphemes)?;
    Ok(Some((Cmd::PageDown(index, window, print_sfx), None)))
}

fn parse_page_up_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    index: Option<usize>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let window = parse_usize(graphemes)?;
    let print_sfx = parse_print_suffix(graphemes)?;
    Ok(Some((Cmd::PageUp(index, window, print_sfx), None)))
}

fn parse_show_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<&Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(Some((Cmd::ShowDiff(None), None))),
        Some(" " | "\t") => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::NoFilename)
            } else {
                Ok(Some((Cmd::ShowDiff(Some(PathBuf::from(filename))), None)))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_edit_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<&Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Err(Error::NoFilename),
        Some(" " | "\t") => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::NoFilename)
            } else {
                Ok(Some((Cmd::Edit(PathBuf::from(filename)), None)))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_replacement_line(
    graphemes: &mut Peekable<Graphemes<'_>>,
    buffer: &EditBuffer,
    replacement: &mut String,
    delimiter: &str,
) -> Result<bool, Error> {
    Ok(loop {
        match graphemes.next() {
            None => break false,
            Some(gr) if gr == delimiter || gr.is_eol() => {
                break false;
            }
            Some("\\") => {
                let escaped =
                    graphemes.next().ok_or(Error::TrailingBackslash)?;
                if escaped.is_eol() {
                    replacement.push_str(buffer.eols().prevailing().into());
                    break true;
                }
                if escaped != delimiter && escaped != "\\" {
                    replacement.push('\\');
                }
                replacement.push_str(escaped);
            }
            Some(gr) => replacement.push_str(gr),
        }
    })
}

fn parse_substitution_scope(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<SubstitutionScope, Error> {
    if let Some(i) = parse_usize(graphemes)? {
        Ok(SubstitutionScope::Single(i))
    } else if let Some(gr) = graphemes.peek()
        && *gr == "g"
    {
        graphemes.next();
        Ok(SubstitutionScope::Global)
    } else {
        Ok(SubstitutionScope::Single(1))
    }
}

pub(crate) fn parse_substitute_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    input: &mut impl LineEdit,
    buffer: &EditBuffer,
    previous_pattern: &mut Option<Regex>,
    address: Option<Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let (pattern, delimiter) = parse_pattern(graphemes, None, true)?;
    if !(pattern.is_empty()) {
        *previous_pattern = Some(
            Regex::new(&pattern)
                .map_err(|e| Error::Regex { source: Some(Box::new(e)) })?,
        );
    }
    let pattern = previous_pattern.clone().ok_or(Error::NoPreviousPattern)?;

    let mut replacement = String::new();
    let more_lines = parse_replacement_line(
        graphemes,
        buffer,
        &mut replacement,
        delimiter.as_str(),
    )?;

    if !more_lines {
        let scope = parse_substitution_scope(graphemes)?;
        let sfx = parse_print_suffix(graphemes)?;
        return Ok(Some((
            Cmd::Substitute(address, pattern, replacement, scope),
            sfx,
        )));
    }

    let line_read_options =
        EditorOptions { prompt: None, history: false, ..Default::default() };
    let mut line = String::new();
    let (cmd, sfx) = loop {
        input
            .read_line(&mut line, Some(&line_read_options))
            .map_err(|e| Error::ReadCommand { source: Some(Box::new(e)) })?;
        let mut graphemes = line.graphemes(true).peekable();
        let more_lines = parse_replacement_line(
            &mut graphemes,
            buffer,
            &mut replacement,
            delimiter.as_str(),
        )?;
        if !more_lines {
            let scope = parse_substitution_scope(&mut graphemes)?;
            let sfx = parse_print_suffix(&mut graphemes)?;
            break (Cmd::Substitute(address, pattern, replacement, scope), sfx);
        }
        line.clear();
    };
    Ok(Some((cmd, sfx)))
}

pub fn parse_pattern(
    graphemes: &mut Peekable<Graphemes<'_>>,
    delimiter: Option<&str>,
    require_closing_delimiter: bool,
) -> Result<(String, String), Error> {
    let delimiter = delimiter
        .map_or_else(
            || {
                graphemes
                    .next_if(|gr| *gr != "\n" && *gr != "\r\n" && *gr != " ")
                    .ok_or(Error::InvalidDelimiter)
            },
            Ok,
        )?
        .to_owned();
    let mut text = String::new();
    loop {
        match graphemes.next_if(|gr| *gr != "\n" && *gr != "\r\n") {
            Some(gr) if gr == delimiter => {
                break;
            }
            None => {
                if require_closing_delimiter {
                    return Err(Error::MissingPatternDelimiter);
                }
                break;
            }
            Some("\\") => {
                let escaped_gr = graphemes
                    .next_if(|gr| *gr != "\n" && *gr != "\r\n")
                    .ok_or(Error::TrailingBackslash)?;
                if escaped_gr != delimiter {
                    text.push('\\');
                }
                text.push_str(escaped_gr);
            }
            Some(gr) => text.push_str(gr),
        }
    }
    Ok((text, delimiter))
}

fn parse_simple_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<&Range<usize>>,
    cmd: Cmd,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    if address.is_some() {
        Err(Error::UnexpectedAddress)
    } else {
        parse_no_args(graphemes, cmd)
    }
}

fn parse_no_args(
    graphemes: &mut Peekable<Graphemes<'_>>,
    cmd: Cmd,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    Ok(Some((cmd, parse_print_suffix(graphemes)?)))
}

pub fn parse_usize(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<Option<usize>, Error> {
    let digits = graphemes
        .peeking_take_while(|s| {
            s.len() == 1 && s.chars().next().unwrap().is_ascii_digit()
        })
        .map(|s| {
            s.chars().next().and_then(|c| c.to_digit(10)).unwrap() as usize
        })
        .try_fold(None, |acc: Option<usize>, d| {
            let v = acc.map_or(Some(d), |a| {
                a.checked_mul(10).and_then(|n| n.checked_add(d))
            });
            v.and(Some(v))
        });

    digits.ok_or(Error::NumberParse)
}

fn parse_global_command_line(
    graphemes: &mut Peekable<Graphemes<'_>>,
    cmd_line: &mut String,
    mut subst_delimiter: String,
) -> Result<(bool, String), Error> {
    if subst_delimiter.is_empty()
        && let Some(gr) = graphemes.peek()
        && *gr == "s"
    {
        // substitute command
        cmd_line.push('s');
        graphemes.next();
        let (pattern, delimiter) = parse_pattern(graphemes, None, true)?;
        cmd_line.push_str(&delimiter);
        cmd_line.push_str(&pattern);
        cmd_line.push_str(&delimiter);
        subst_delimiter = delimiter;
    }

    if !subst_delimiter.is_empty() {
        // We're reading a substitute replacement string
        for gr in graphemes.by_ref() {
            cmd_line.push_str(gr);
            if gr == subst_delimiter {
                subst_delimiter.clear();
                break;
            } else if gr == "\r\n" || gr == "\n" {
                return Ok((true, subst_delimiter));
            }
        }
    }

    Ok(loop {
        let gr = graphemes.next();
        match gr {
            None => break (false, subst_delimiter),
            Some("\\") => {
                let escaped =
                    graphemes.next().ok_or(Error::TrailingBackslash)?;
                if escaped == "\n" || escaped == "\r\n" {
                    cmd_line.push_str(escaped);
                    break (true, subst_delimiter);
                }

                cmd_line.push('\\');
                cmd_line.push_str(escaped);
            }
            Some(gr) => {
                cmd_line.push_str(gr);
                if gr == "\r\n" || gr == "\n" {
                    break (false, subst_delimiter);
                }
            }
        }
    })
}

fn parse_global_command_list(
    cmd_line: &mut Peekable<Graphemes<'_>>,
    input: &mut impl LineEdit,
) -> Result<String, Error> {
    let mut commands = String::new();
    // Copy first command to commands string,
    // noting and unescaping escaped EOL.
    let (mut more_lines, mut subst_delimiter) =
        parse_global_command_line(cmd_line, &mut commands, String::new())?;

    if more_lines {
        let line_read_options = EditorOptions {
            prompt: None,
            history: false,
            ..Default::default()
        };
        let mut line = String::new();
        while more_lines {
            input.read_line(&mut line, Some(&line_read_options)).map_err(
                |e| Error::ReadCommand { source: Some(Box::new(e)) },
            )?;
            let mut graphemes = line.graphemes(true).peekable();
            (more_lines, subst_delimiter) = parse_global_command_line(
                &mut graphemes,
                &mut commands,
                subst_delimiter,
            )?;
            line.clear();
        }
    }
    Ok(commands)
}

fn parse_global_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    input: &mut impl LineEdit,
    previous_pattern: &mut Option<Regex>,
    address: Option<Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let (pattern, _) = parse_pattern(graphemes, None, false)?;
    if !(pattern.is_empty()) {
        *previous_pattern = Some(
            Regex::new(&pattern)
                .map_err(|e| Error::Regex { source: Some(Box::new(e)) })?,
        );
    }
    let pattern = previous_pattern.clone().ok_or(Error::NoPreviousPattern)?;

    let commands = parse_global_command_list(graphemes, input)?;

    Ok(Some((Cmd::Global(address, pattern, commands), None)))
}

fn parse_join_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    let Some(next_gr) = graphemes.peek() else {
        return Ok(Some((Cmd::Join(address, None), None)));
    };
    let (cmd, sfx) = match *next_gr {
        "l" | "n" | "p" => {
            let sfx = parse_print_suffix(graphemes)?;
            (Cmd::Join(address, None), sfx)
        }
        "\r\n" | "\r" => (Cmd::Join(address, None), None),
        d => {
            graphemes.next();
            let (separator, _) = parse_pattern(graphemes, Some(d), false)?;
            let sfx = parse_print_suffix(graphemes)?;
            (Cmd::Join(address, Some(separator)), sfx)
        }
    };
    Ok(Some((cmd, sfx)))
}

fn parse_justify_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    span: Option<Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    // Parse optional wrapping style
    let wrap = match graphemes.peek() {
        Some(&"/") => {
            graphemes.next();
            Wrapping::NoFill
        }
        Some(&"^") => {
            graphemes.next();
            Wrapping::Fill
        }
        Some(&"!") => {
            graphemes.next();
            Wrapping::None
        }
        _ => Wrapping::default(),
    };

    // Parse optional left margin
    let left_margin = parse_usize(graphemes)?;

    // Parse optional right margin
    while graphemes.next_if(|&gr| gr == " " || gr == "\t").is_some() {}
    let line_width = parse_usize(graphemes)?;

    // Parse optional print suffix
    let pr_sfx = parse_print_suffix(graphemes)?;
    Ok(Some((Cmd::Justify { span, wrap, left_margin, line_width }, pr_sfx)))
}

fn parse_newline_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<&Range<usize>>,
) -> Result<Option<(Cmd, Option<PrintSuffix>)>, Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(Some((Cmd::Newline(None), None))),
        Some(" " | "\t") => {
            let eol = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if eol.is_empty() {
                Ok(Some((Cmd::Newline(None), None)))
            } else {
                Ok(Some((
                    Cmd::Newline(Some(
                        eol.parse::<Eol>()
                            .map_err(|_| Error::InvalidNewline)?,
                    )),
                    None,
                )))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum Delimiter {
    Fwd,
    Rev,
}

// Evaluates an address.
//
// Depending upon the address string, `eval_address`
// may update `previous_pattern` and/or the buffer's
// current index.
//
// Returns the inclusive range of line indices specified
// by the address, or None if no address specified.
// Returns an appropriate `Error` if the address is
// invalid in some way.
fn eval_address(
    graphemes: &mut Peekable<Graphemes<'_>>,
    buffer: &mut EditBuffer,
    previous_pattern: &mut Option<Regex>,
) -> Result<Option<Range<usize>>, Error> {
    let mut left = None;
    let mut right = None;

    loop {
        match graphemes.peek() {
            Some(&",") => {
                graphemes.next();
                left = right.or(Some(1));
                right = right.or_else(|| Some(buffer.len()));
            }
            Some(&";") => {
                graphemes.next();
                left = Some(match right {
                    Some(r) if r > buffer.len() => {
                        return Err(Error::InvalidAddress);
                    }
                    Some(r) => {
                        buffer.set_current_index(r - 1);
                        r
                    }
                    None => buffer.current_index() + 1,
                });
                right = right.or_else(|| Some(buffer.len()));
            }
            Some(&"+" | &"-") => {
                right = Some(eval_line_number(
                    graphemes,
                    buffer.current_index() + 1,
                )?);
            }
            Some(&".") => {
                graphemes.next();
                right = Some(eval_line_number(
                    graphemes,
                    buffer.current_index() + 1,
                )?);
            }
            Some(&"$") => {
                graphemes.next();
                right = Some(eval_line_number(graphemes, buffer.len())?);
            }
            Some(&delim) if delim == <&str>::from(Delimiter::Fwd) => {
                graphemes.next();
                let line = eval_pattern(
                    graphemes,
                    Delimiter::Fwd,
                    buffer,
                    previous_pattern,
                )?;
                right = Some(eval_line_number(graphemes, line)?);
            }
            Some(&delim) if delim == <&str>::from(Delimiter::Rev) => {
                graphemes.next();
                let line = eval_pattern(
                    graphemes,
                    Delimiter::Rev,
                    buffer,
                    previous_pattern,
                )?;
                right = Some(eval_line_number(graphemes, line)?);
            }
            Some(&" " | &"\t") => {
                graphemes.next();
            }
            Some(_) => {
                if let Some(num) = parse_usize(graphemes)? {
                    right = Some(eval_line_number(graphemes, num)?);
                } else {
                    break;
                }
            }
            None => break,
        }
        if left.is_none() && right.is_some() {
            left = right;
        }
    }

    if let Some(last) = right {
        if buffer.is_empty() {
            return Err(Error::InvalidAddress);
        }
        let first = left.unwrap_or(last);
        if first == 0 || first > last || last > buffer.len() {
            return Err(Error::InvalidAddress);
        }
        Ok(Some(first - 1..last))
    } else {
        Ok(None)
    }
}

impl From<Delimiter> for &'static str {
    fn from(value: Delimiter) -> Self {
        match value {
            Delimiter::Fwd => "/",
            Delimiter::Rev => "?",
        }
    }
}

fn eval_pattern(
    graphemes: &mut Peekable<Graphemes<'_>>,
    delimiter: Delimiter,
    buffer: &EditBuffer,
    previous_pattern: &mut Option<Regex>,
) -> Result<usize, Error> {
    let (pattern, _) = parse_pattern(graphemes, Some(delimiter.into()), false)?;
    if !pattern.is_empty() {
        *previous_pattern = Some(
            Regex::new(&pattern)
                .map_err(|e| Error::Regex { source: Some(Box::new(e)) })?,
        );
    }
    let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
    Ok(match delimiter {
        Delimiter::Fwd => find_line(buffer, re).ok_or(Error::NoMatch)?,
        Delimiter::Rev => find_line_rev(buffer, re).ok_or(Error::NoMatch)?,
    })
}

fn eval_line_number(
    graphemes: &mut Peekable<Graphemes<'_>>,
    line: usize,
) -> Result<usize, Error> {
    let offset = compute_line_offset(graphemes)?;
    line.checked_add_signed(offset).ok_or(Error::InvalidOffset)
}

fn compute_line_offset(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<isize, Error> {
    let mut total_offset = 0isize;
    while let Some(n) = parse_offset_element(graphemes)? {
        total_offset =
            total_offset.checked_add(n).ok_or(Error::InvalidOffset)?;
    }
    Ok(total_offset)
}

fn parse_offset_element(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<Option<isize>, Error> {
    // Skip leading whitespace
    while graphemes.peek().is_some_and(|s| *s == " " || *s == "\t") {
        graphemes.next();
    }

    let sign = graphemes
        .next_if(|c| *c == "+" || *c == "-")
        .map(|c| if c == "-" { -1 } else { 1 });

    let sign_mul = sign.unwrap_or(1);

    let digits = graphemes
        .peeking_take_while(|s| {
            s.len() == 1 && s.chars().next().unwrap().is_ascii_digit()
        })
        .map(|s| {
            isize::try_from(
                s.chars()
                    .next()
                    .and_then(|c| c.to_digit(10))
                    .expect("ascii 0-9"),
            )
            .expect("0-9 always fit isize")
        })
        .try_fold(None, |acc: Option<isize>, d| {
            let v = acc.map_or(Some(sign_mul * d), |a| {
                a.checked_mul(10).and_then(|n| n.checked_add(sign_mul * d))
            });
            v.and(Some(v))
        });

    Ok(digits.ok_or(Error::InvalidOffset)?.or(sign))
}

fn find_line(buffer: &EditBuffer, pattern: &Regex) -> Option<usize> {
    let index = if buffer.current_index() == buffer.len() - 1 {
        (0..buffer.len()).find(|&i| pattern.is_match(Eol::strip(&buffer[i])))
    } else {
        (buffer.current_index() + 1..buffer.len())
            .find(|&i| pattern.is_match(Eol::strip(&buffer[i])))
            .or_else(|| {
                (0..=buffer.current_index())
                    .find(|&i| pattern.is_match(Eol::strip(&buffer[i])))
            })
    };
    index.map(|i| i + 1)
}

fn find_line_rev(buffer: &EditBuffer, pattern: &Regex) -> Option<usize> {
    let index = if buffer.current_index() == 0 {
        (0..buffer.len())
            .rev()
            .find(|&i| pattern.is_match(Eol::strip(&buffer[i])))
    } else {
        (0..buffer.current_index())
            .rev()
            .find(|&i| pattern.is_match(Eol::strip(&buffer[i])))
            .or_else(|| {
                (buffer.current_index()..buffer.len())
                    .rev()
                    .find(|&i| pattern.is_match(Eol::strip(&buffer[i])))
            })
    };
    index.map(|i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    use similar_asserts::assert_eq;

    use crate::eol::Eol;

    #[test]
    fn parse_valid_lone_cmd() {
        let mut cmd_line = "\r\n".graphemes(true).peekable();
        let res = parse_simple_cmd(&mut cmd_line, None, Cmd::Quit).unwrap();
        assert!(matches!(res, Some((Cmd::Quit, None))));
    }

    #[test]
    fn parse_simple_cmd_error_with_address() {
        let address = Some(0..1);
        let mut cmd_line = "\r\n".graphemes(true).peekable();
        let res = parse_simple_cmd(&mut cmd_line, address.as_ref(), Cmd::Quit)
            .expect_err("unexpected address");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_no_args_error_with_extra_chars() {
        let mut cmd_line = "extra\n".graphemes(true).peekable();
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None))
            .expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_no_args_both_line_terminators_valid() {
        let mut cmd_line = "\n".graphemes(true).peekable();
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        assert!(matches!(res, Some((Cmd::Delete(None), None))));
    }

    #[test]
    fn parse_no_args_p_print_suffix() {
        let mut cmd_line = "p\r\n".graphemes(true).peekable();
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        let expected_sfx = Some(PrintSuffix { ..Default::default() });
        assert!(
            matches!(res, Some((Cmd::Delete(None), pr_sfx)) if pr_sfx == expected_sfx)
        );
    }

    #[test]
    fn parse_no_args_n_print_suffix() {
        let mut cmd_line = "n\r\n".graphemes(true).peekable();
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        let expected_sfx =
            Some(PrintSuffix { enumerate: true, ..Default::default() });
        assert!(
            matches!(res, Some((Cmd::Delete(None), pr_sfx)) if pr_sfx == expected_sfx)
        );
    }

    #[test]
    fn parse_no_args_extra_chars_after_print_suffix_error() {
        let mut cmd_line = "n!\r\n".graphemes(true).peekable();
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None))
            .expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_print_suffix_p() {
        let mut graphs = "p\r\n".graphemes(true).peekable();
        let res = parse_print_suffix(&mut graphs).unwrap();
        assert!(matches!(
            res,
            Some(a) if a == PrintSuffix { ..Default::default() }));
    }

    #[test]
    fn parse_print_suffix_n() {
        let mut graphs = "n\r\n".graphemes(true).peekable();
        let res = parse_print_suffix(&mut graphs).unwrap();
        assert!(matches!(
            res,
            Some(a) if a == PrintSuffix {
                enumerate: true,
                ..Default::default()
            }
        ));
    }

    #[test]
    fn parse_print_suffix_l() {
        let mut graphs = "l\r\n".graphemes(true).peekable();
        let res = parse_print_suffix(&mut graphs).unwrap();
        assert!(matches!(
        res,
        Some(a) if a ==PrintSuffix {
            expand_escapes: true,
            ..Default::default()
        }));
    }

    #[test]
    fn parse_print_suffix_trailing_chars_error() {
        let mut graphs = "pn5\r\n".graphemes(true).peekable();
        let res = parse_print_suffix(&mut graphs).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn eval_no_addr_null_cmd() {
        let mut cmd_line = "\r\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn eval_no_addr_null_cmd_skip_spaces() {
        let mut cmd_line = "\t  \r\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn parse_pattern_delimiter_invalid() {
        let mut input = " stuff + other_stuff. \n".graphemes(true).peekable();
        let res = parse_pattern(&mut input, None, false);
        assert!(matches!(res, Err(Error::InvalidDelimiter)));
    }

    #[test]
    fn parse_pattern_trailing_backslash() {
        let mut input = "/stuff + other_stuff.\\\n".graphemes(true).peekable();
        let res = parse_pattern(&mut input, None, false)
            .expect_err("trailing backslash");
        assert!(matches!(res, Error::TrailingBackslash));
        let mut input = "/stuff + other_stuff.\\".graphemes(true).peekable();
        let res = parse_pattern(&mut input, None, false)
            .expect_err("trailing backslash");
        assert!(matches!(res, Error::TrailingBackslash));
    }

    #[test]
    fn parse_pattern_no_terminating_delimiter() {
        let mut input = "/stuff\\/other_stuff.\n".graphemes(true).peekable();
        let (pattern, _) =
            parse_pattern(&mut input.clone(), None, false).unwrap();
        assert_eq!("stuff/other_stuff.".to_owned(), pattern);
        let res = parse_pattern(&mut input, None, true)
            .expect_err("missing delimiter");
        assert!(matches!(res, Error::MissingPatternDelimiter));
    }

    #[test]
    fn parse_pattern_escaped_terminator() {
        let mut input = "/stuff\\/other_stuff./\n".graphemes(true).peekable();
        let (res, _) = parse_pattern(&mut input, None, true).unwrap();
        assert_eq!("stuff/other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_escaped_chars() {
        let mut input =
            "?stuff \\+ other_stuff\\.?\n".graphemes(true).peekable();
        let (res, _) = parse_pattern(&mut input, None, false).unwrap();
        assert_eq!("stuff \\+ other_stuff\\.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_no_escaped_chars() {
        let mut input = "/stuff + other_stuff./\n".graphemes(true).peekable();
        let (res, _) = parse_pattern(&mut input, None, false).unwrap();
        assert_eq!("stuff + other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_append_cmd_no_addr() {
        let mut input = "a\r\n".as_bytes();
        let (cmd, pr_sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
                .expect("should be Ok")
                .expect("should be (cmd, pr_sfx)");
        assert!(matches!(
            cmd,
            Cmd::Append {
                index: None,
                source: InputSource::StdIn,
                mode: InputMode::Cooked
            }
        ));
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_append_cmd_with_addr() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2a\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("should be Ok")
            .expect("should be (cmd, pr_sfx)");
        let Cmd::Append { index, source, mode } = cmd else {
            panic!("expected Cmd::Append");
        };
        assert_eq!(index, Some(1));
        assert_eq!(source, InputSource::StdIn);
        assert_eq!(mode, InputMode::Cooked);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_append_cmd_from_clipboard() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2Av\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("should be Ok")
            .expect("should be (cmd, pr_sfx)");
        let Cmd::Append { index, source, mode } = cmd else {
            panic!("expected Cmd::Append");
        };
        assert_eq!(index, Some(1));
        assert_eq!(source, InputSource::Clipboard);
        assert_eq!(mode, InputMode::Raw);
        assert!(pr_sfx.is_none());
    }
    #[test]
    fn parse_append_cmd_from_file() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2A filename\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("should be Ok")
            .expect("should be (cmd, pr_sfx)");
        let Cmd::Append { index, source, mode } = cmd else {
            panic!("expected Cmd::Append");
        };
        assert_eq!(index, Some(1));
        assert_eq!(source, InputSource::File(PathBuf::from("filename")));
        assert_eq!(mode, InputMode::Raw);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_delete_cmd_no_addr() {
        let mut input = "d\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Delete(None), None))),);
    }

    #[test]
    fn parse_enumerate_cmd_no_addr() {
        let mut input = "n\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Enumerate(None), None))),);
    }

    #[test]
    fn parse_insert_cmd_no_addr() {
        let mut input = "i\r\n".as_bytes();
        let (cmd, pr_sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
                .expect("no error")
                .expect("parsed (cmd, pr_sfx)");
        assert!(matches!(
            cmd,
            Cmd::Insert {
                index: None,
                source: InputSource::StdIn,
                mode: InputMode::Cooked
            }
        ));
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_insert_cmd_with_addr() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2i\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("no error")
            .expect("parsed (cmd, pr_sfx)");
        let Cmd::Insert { index, source, mode } = cmd else {
            panic!("should parse to Cmd::Insert");
        };
        assert_eq!(index, Some(1));
        assert_eq!(source, InputSource::StdIn);
        assert_eq!(mode, InputMode::Cooked);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_insert_cmd_from_clipboard() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2Iv\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("no error")
            .expect("parsed (cmd, pr_sfx)");
        let Cmd::Insert { index, source, mode } = cmd else {
            panic!("should parse to Cmd::Insert");
        };
        assert_eq!(index, Some(1));
        assert_eq!(source, InputSource::Clipboard);
        assert_eq!(mode, InputMode::Raw);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_insert_cmd_from_file() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2I filename\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("should be Ok")
            .expect("should be (cmd, pr_sfx)");
        let Cmd::Insert { index, source, mode } = cmd else {
            panic!("expected Cmd::Insert");
        };
        assert_eq!(index, Some(1));
        assert_eq!(source, InputSource::File(PathBuf::from("filename")));
        assert_eq!(mode, InputMode::Raw);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_overwrite_cmd_no_addr() {
        let mut input = "o\r\n".as_bytes();
        let (cmd, pr_sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
                .expect("no error")
                .expect("parsed (cmd, pr_sfx)");
        let Cmd::Overwrite { span, source, mode } = cmd else {
            panic!("expected Cmd::Overwrite");
        };
        assert!(span.is_none());
        assert_eq!(source, InputSource::StdIn);
        assert_eq!(mode, InputMode::Cooked);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_overwrite_cmd_with_addr() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2o\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("no error")
            .expect("parsed (cmd, pr_sfx)");
        let Cmd::Overwrite { span, source, mode } = cmd else {
            panic!("should parse to Cmd::Overwrite");
        };
        assert_eq!(span, Some(1..2));
        assert_eq!(source, InputSource::StdIn);
        assert_eq!(mode, InputMode::Cooked);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_overwrite_cmd_from_clipboard() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2Ov\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("no error")
            .expect("parsed (cmd, pr_sfx)");
        let Cmd::Overwrite { span, source, mode } = cmd else {
            panic!("should parse to Cmd::Overwrite");
        };
        assert_eq!(span, Some(1..2));
        assert_eq!(source, InputSource::Clipboard);
        assert_eq!(mode, InputMode::Raw);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_overwrite_cmd_from_file() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut input = "2O filename\r\n".as_bytes();
        let (cmd, pr_sfx) = Cmd::read(&mut input, &mut buffer, &mut None)
            .expect("should be Ok")
            .expect("should be (cmd, pr_sfx)");
        let Cmd::Overwrite { span, source, mode } = cmd else {
            panic!("expected Cmd::Overwrite");
        };
        assert_eq!(span, Some(1..2));
        assert_eq!(source, InputSource::File(PathBuf::from("filename")));
        assert_eq!(mode, InputMode::Raw);
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_null_cmd_no_addr() {
        let mut input = "\r\n".as_bytes();
        let (cmd, pr_sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
                .expect("no error")
                .expect("parsed (cmd, pr_sfx)");
        let Cmd::Null(index) = cmd else {
            panic!("cmd wasn't Null(index)!");
        };
        assert!(index.is_none());
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_print_cmd_no_addr() {
        let mut input = "p\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Print(None), None))),);
    }

    #[test]
    fn parse_quit_cmd() {
        let mut input = "q\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Quit, None))));
    }

    #[test]
    fn parse_undo_cmd() {
        let mut input = "u\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Undo, None))));
    }

    #[test]
    fn parse_redo_cmd() {
        let mut input = "U\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Redo, None))));
    }

    #[test]
    fn parse_quit_cmd_invalid_suffix() {
        let mut input = "q/more stuff/\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
            .expect_err("invalid sufix");
        assert!(
            matches!(res, Error::InvalidCmdSuffix),
            "{res:?} didn't match Error::InvalidCmdSuffix"
        );
    }

    #[test]
    fn parse_unknown_command() {
        let mut input = "*\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
            .expect_err("unknown cmd");
        assert!(
            matches!(res, Error::UnknownCmd(ref s) if s == "*"),
            "{res:?} didn't match Error::UnknownCmd(\"O\")"
        );
    }

    #[test]
    fn parse_open_no_print_suffix() {
        let mut cmd_line = " filename.rs".graphemes(true).peekable();
        let res = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(res, Some((_, None))));
    }

    #[test]
    fn parse_open_with_address() {
        let address = Some(0..1);
        let mut cmd_line = " filename.rs".graphemes(true).peekable();
        let res = parse_edit_cmd(&mut cmd_line, address.as_ref())
            .expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_open_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true).peekable();
        let res =
            parse_edit_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn parse_open_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true).peekable();
        let res = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&res, Some((Cmd::Edit(f), None)) if f.to_str().unwrap() == "a/filename.rs")
        );
    }

    #[test]
    fn parse_open_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true).peekable();
        let res =
            parse_edit_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_global_simple_cmd() {
        let mut input = "/pat/p\r\n".graphemes(true).peekable();
        let mut prev_pattern = None;
        let res = parse_global_cmd(
            &mut input,
            &mut "".as_bytes(),
            &mut prev_pattern,
            None,
        )
        .unwrap();
        let Some((Cmd::Global(addr, pat, cmds), None)) = res else {
            panic!("{res:?} not Cmd::Global!");
        };
        assert!(addr.is_none());
        assert_eq!(pat.as_str(), "pat");
        assert_eq!(cmds, "p\r\n");
    }

    #[test]
    fn parse_global_substitute_cmd() {
        let mut input_1 = "/pat/s//replacement\n".graphemes(true).peekable();
        let mut input_2 = "".as_bytes();
        let mut prev_pattern = None;
        let res = parse_global_cmd(
            &mut input_1,
            &mut input_2,
            &mut prev_pattern,
            None,
        )
        .unwrap();
        let Some((Cmd::Global(addr, pat, cmds), None)) = res else {
            panic!("{res:?} not Cmd::Global!");
        };
        assert!(addr.is_none());
        assert_eq!(pat.as_str(), "pat");
        assert_eq!(cmds, "s//replacement\n");
    }

    #[test]
    fn parse_global_substitute_cmd_escaped_eol() {
        let mut input_1 = "/pattern/s//pat-\\\n".graphemes(true).peekable();
        let mut input_2 = "tern/n\n".as_bytes();
        let mut prev_pattern = None;
        let res = parse_global_cmd(
            &mut input_1,
            &mut input_2,
            &mut prev_pattern,
            None,
        )
        .unwrap();
        let Some((Cmd::Global(addr, pat, cmds), None)) = res else {
            panic!("{res:?} not Cmd::Global!");
        };
        assert!(addr.is_none());
        assert_eq!(pat.as_str(), "pattern");
        assert_eq!(cmds, "s//pat-\\\ntern/n\n");
    }

    #[test]
    fn parse_global_multi_cmd() {
        let mut input = "/pat/n\\\r\n".graphemes(true).peekable();
        let mut more_input = "d\r\n".as_bytes();
        let mut prev_pattern = None;
        let res = parse_global_cmd(
            &mut input,
            &mut more_input,
            &mut prev_pattern,
            None,
        )
        .unwrap();
        let Some((Cmd::Global(addr, pat, cmds), None)) = res else {
            panic!("{res:?} not Cmd::Global!");
        };
        assert!(addr.is_none());
        assert_eq!(pat.as_str(), "pat");
        assert_eq!(cmds, "n\r\nd\r\n");
    }

    #[test]
    fn parse_default_substitute() {
        let mut cmd_line = "/[^01]*/./n\r\n".graphemes(true).peekable();
        let buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let address = Some(0..5);
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            address.clone(),
        )
        .unwrap();
        let (cmd, pr_sfx) = res.unwrap();
        let expected_sfx =
            PrintSuffix { enumerate: true, ..Default::default() };
        assert!(
            matches!(cmd, Cmd::Substitute(a, p, r, SubstitutionScope::Global) if a == address && p.as_str() == "[^01]*" && r == ".")
        );
        assert!(matches!(pr_sfx, Some(a) if a == expected_sfx));
    }

    #[test]
    fn parse_substitute_global() {
        let mut cmd_line = "/[^01]*/./g\r\n".graphemes(true).peekable();
        let buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let address = Some(0..5);
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            address.clone(),
        )
        .unwrap();
        let Some((Cmd::Substitute(a, p, r, SubstitutionScope::Global), None)) =
            res
        else {
            panic!("Not Global!");
        };
        assert_eq!(a, address);
        assert_eq!(p.as_str(), "[^01]*");
        assert_eq!(r, ".");
    }

    #[test]
    fn parse_substitute_global_escaped_lf() {
        let mut cmd_line = "/, */,\\\n".graphemes(true).peekable();
        let buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let address = Some(0..5);
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "/g\n".as_bytes(),
            &buffer,
            &mut prev_pattern,
            address.clone(),
        )
        .unwrap();
        let Some((Cmd::Substitute(a, p, r, s), None)) = res else {
            panic!("Expected Cmd::Substitute, got {res:?}");
        };
        assert!(
            matches!(s, SubstitutionScope::Global),
            "expected SubstitutionScope::Global, got {s:?}"
        );
        assert_eq!(a, address);
        assert_eq!(p.as_str(), ", *");
        assert_eq!(r, ",\n");
    }

    #[test]
    fn parse_substitute_indexed() {
        let mut cmd_line = "/[^01]*/./3\r\n".graphemes(true).peekable();
        let buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            Some(0..5),
        )
        .unwrap();
        let Some((
            Cmd::Substitute(a, p, r, SubstitutionScope::Single(3)),
            None,
        )) = res
        else {
            panic!("not Single(3)!");
        };
        assert_eq!(a, Some(0..5));
        assert_eq!(p.as_str(), "[^01]*");
        assert_eq!(r, ".");
    }

    #[test]
    fn parse_substitute_conflicting_flags() {
        let mut cmd_line = "/[^01]*/./g1\r\n".graphemes(true).peekable();
        let buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            Some(0..5),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::InvalidCmdSuffix));

        let mut cmd_line = "/[^01]*/./4g\n".graphemes(true).peekable();
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            Some(0..5),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_substitute_invalid_flag() {
        let mut cmd_line = "/[^01]*/./q\r\n".graphemes(true).peekable();
        let buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            Some(0..5),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::InvalidCmdSuffix));

        let mut cmd_line = "/[^01]*/./gq\n".graphemes(true).peekable();
        let res = parse_substitute_cmd(
            &mut cmd_line,
            &mut "".as_bytes(),
            &buffer,
            &mut prev_pattern,
            Some(0..5),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_join_cmd_no_addr() {
        let mut input = "j\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Join(None, None), None))));
    }

    #[test]
    fn parse_list_cmd_no_addr() {
        let mut input = "l\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::List(None), None))));
    }

    #[test]
    fn parse_line_number_cmd() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3", "4"]);
        buffer.set_current_index(1);
        let mut input1 = "=\n".as_bytes();
        let mut input2 = ".=\n".as_bytes();
        let res = Cmd::read(&mut input1, &mut buffer, &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::LineNumber(None), None))));
        let res = Cmd::read(&mut input2, &mut buffer, &mut None).unwrap();
        assert!(
            matches!(res, Some((Cmd::LineNumber(Some(n)), None)) if n == 1)
        );
    }

    #[test]
    fn parse_write_as_cmd_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let res = parse_write_as_cmd(&mut cmd_line, Some(0..5)).unwrap();
        assert!(
            matches!(res, Some((Cmd::WriteAs(span, f), None)) if span == Some(0..5) && f.to_str().unwrap() == "filename.rs")
        );
    }

    #[test]
    fn parse_write_as_cmd_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res =
            parse_write_as_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn parse_write_as_cmd_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_write_as_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&res, Some((Cmd::WriteAs(None, f), None)) if f.to_str().unwrap() == "a/filename.rs"),
            "{res:?} wasnt Cmd::WriteAs('filename.rs')"
        );
    }

    #[test]
    fn parse_write_as_cmd_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_write_as_cmd(&mut cmd_line, None)
            .expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn show_cmd_no_args_parses_as_show_diff_cmd() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_show_cmd(&mut cmd_line, None).expect("show diff cmd");
        assert!(matches!(res, Some((Cmd::ShowDiff(None), None))));
    }

    #[test]
    fn show_cmd_with_filename_parses() {
        let mut cmd_line = " filename.txt\n".graphemes(true);
        let res = parse_show_cmd(&mut cmd_line, None).expect("show diff cmd");
        let filename = PathBuf::from(r"filename.txt");
        assert!(
            matches!(res, Some((Cmd::ShowDiff(Some(path)), None)) if path == filename)
        );
    }

    #[test]
    fn show_cmd_with_address_fails() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_show_cmd(&mut cmd_line, Some(&(0..1)))
            .expect_err("unexpected address");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn show_cmd_with_bad_filename_fails() {
        let mut cmd_line = " \n".graphemes(true);
        let res =
            parse_show_cmd(&mut cmd_line, None).expect_err("invalid filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn show_cmd_with_suffix_fails() {
        let mut cmd_line = "n\n".graphemes(true);
        let res =
            parse_show_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_version_cmd() {
        let mut input = "#\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Some((Cmd::Version, None))));
    }

    #[test]
    fn parse_valid_page_down_cmd() {
        let mut input = "5z10\n".as_bytes();
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let res = Cmd::read(&mut input, &mut buffer, &mut None).unwrap();
        assert!(
            matches!(res, Some((Cmd::PageDown(i, w, p), None)) if i == Some(4) && w == Some(10) && p.is_none())
        );
    }

    #[test]
    fn parse_valid_page_up_cmd() {
        let mut input = "5Z10\n".as_bytes();
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let res = Cmd::read(&mut input, &mut buffer, &mut None).unwrap();
        assert!(
            matches!(res, Some((Cmd::PageUp(i, w, p), None)) if i == Some(4) && w == Some(10) && p.is_none())
        );
    }

    #[test]
    fn parse_newline_no_print_suffix() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_newline_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(res, Some((_, None))));
    }

    #[test]
    fn parse_newline_cmd_with_address() {
        let mut cmd_line = " CRLF".graphemes(true);
        let res = parse_newline_cmd(&mut cmd_line, Some(&(0..1)))
            .expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_newline_cmd_no_newline() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_newline_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(res, Some((Cmd::Newline(None), None))));
    }

    #[test]
    fn parse_newline_cmd_bad_newline() {
        let mut cmd_line = " HT\r\n".graphemes(true);
        let res = parse_newline_cmd(&mut cmd_line, None).expect_err("bad eol");
        assert!(matches!(res, Error::InvalidNewline));
    }

    #[test]
    fn parse_newline_cmd_with_newline() {
        let mut cmd_line = " LF\n".graphemes(true);
        let res = parse_newline_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(&res, Some((Cmd::Newline(Some(Eol::Lf)), None))),);
    }

    #[test]
    fn parse_newline_cmd_invalid_suffix() {
        let mut cmd_line = "LF\n".graphemes(true);
        let res =
            parse_newline_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_justify_cmd_with_addr() {
        let expected_addr = 1..5;
        let expected_sfx =
            PrintSuffix { enumerate: true, ..Default::default() };

        // No line_width, no print_suffix
        let mut input = "n\n".graphemes(true).peekable();

        let (cmd, pr_sfx) =
            parse_justify_cmd(&mut input, Some(expected_addr.clone()))
                .expect("no error")
                .expect("should parse");
        let Cmd::Justify { span, wrap, left_margin, line_width } = cmd else {
            panic!("expected Cmd::Justify, got {cmd:?}");
        };
        assert_eq!(span, Some(expected_addr));
        assert_eq!(wrap, Wrapping::Fill);
        assert!(left_margin.is_none());
        assert!(line_width.is_none());
        assert_eq!(pr_sfx, Some(expected_sfx));
    }

    #[test]
    fn parse_justify_cmd_left_margin_no_wrap() {
        let expected_addr = 1..5;
        let expected_sfx =
            PrintSuffix { enumerate: true, ..Default::default() };

        // No line_width, no print_suffix
        let mut input = "!10n\n".graphemes(true).peekable();

        let (cmd, pr_sfx) =
            parse_justify_cmd(&mut input, Some(expected_addr.clone()))
                .expect("no error")
                .expect("should parse");
        let Cmd::Justify { span, wrap, left_margin, line_width } = cmd else {
            panic!("expected Cmd::Justify, got {cmd:?}");
        };
        assert_eq!(span, Some(expected_addr));
        assert_eq!(wrap, Wrapping::None);
        assert_eq!(left_margin, Some(10));
        assert!(line_width.is_none());
        assert_eq!(pr_sfx, Some(expected_sfx));
    }

    #[test]
    fn parse_justify_cmd_default() {
        let mut input = "\n".graphemes(true).peekable();

        let (cmd, pr_sfx) = parse_justify_cmd(&mut input, None)
            .expect("no error")
            .expect("should parse");
        let Cmd::Justify { span, wrap, left_margin, line_width } = cmd else {
            panic!("expected Cmd::Justify, got {cmd:?}");
        };
        assert!(span.is_none());
        assert_eq!(wrap, Wrapping::Fill);
        assert!(left_margin.is_none());
        assert!(line_width.is_none());
        assert!(pr_sfx.is_none());
    }

    #[test]
    fn parse_justify_cmd_all_parameters() {
        let mut input = "^20 72\n".graphemes(true).peekable();

        let (cmd, pr_sfx) = parse_justify_cmd(&mut input, None)
            .expect("no error")
            .expect("should parse");
        let Cmd::Justify { span, wrap, left_margin, line_width } = cmd else {
            panic!("expected Cmd::Justify, got {cmd:?}");
        };
        assert!(span.is_none());
        assert_eq!(wrap, Wrapping::Fill);
        assert_eq!(left_margin, Some(20));
        assert_eq!(line_width, Some(72));
        assert!(pr_sfx.is_none());
    }
}
