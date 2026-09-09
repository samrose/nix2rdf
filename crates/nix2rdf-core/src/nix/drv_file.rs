//! Optional fast path: parse `.drv` files (ATerm format) directly instead of
//! calling `nix derivation show`. Must produce a `DrvInfo` identical to the
//! CLI's; the determinism test enforces it. Enabled with `--reader drv-files`.
//!
//! Format: `Derive([outputs],[inputDrvs],[inputSrcs],"system","builder",[args],[env])`
//! - outputs:   `("out","/nix/store/...","","")` or, for content-addressed
//!   derivations, `("out","","r:sha256","")` / `("out","","sha256","<hash>")`.
//! - inputDrvs: `("/nix/store/x.drv",["out","dev"])`.
//! - env:       `("NAME","value")`.

use super::{DrvInfo, InputDrv, OutputInfo};
use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
enum Term {
    Str(String),
    List(Vec<Term>),
    Tuple(Vec<Term>),
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, b: u8) -> Result<()> {
        if self.peek() == Some(b) {
            self.i += 1;
            Ok(())
        } else {
            Err(Error::Other(format!(
                "drv parse: expected '{}' at {}",
                b as char, self.i
            )))
        }
    }
    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(Error::Other("drv parse: unterminated string".into())),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self
                        .peek()
                        .ok_or_else(|| Error::Other("drv parse: bad escape".into()))?;
                    self.i += 1;
                    out.push(match c {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        other => other,
                    });
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        String::from_utf8(out).map_err(|e| Error::Other(format!("drv parse: {e}")))
    }
    fn term(&mut self) -> Result<Term> {
        match self.peek() {
            Some(b'"') => Ok(Term::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                Ok(Term::List(self.seq(b']')?))
            }
            Some(b'(') => {
                self.i += 1;
                Ok(Term::Tuple(self.seq(b')')?))
            }
            other => Err(Error::Other(format!(
                "drv parse: unexpected {:?} at {}",
                other.map(|b| b as char),
                self.i
            ))),
        }
    }
    fn seq(&mut self, close: u8) -> Result<Vec<Term>> {
        let mut items = Vec::new();
        if self.peek() == Some(close) {
            self.i += 1;
            return Ok(items);
        }
        loop {
            items.push(self.term()?);
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(c) if c == close => {
                    self.i += 1;
                    return Ok(items);
                }
                _ => {
                    return Err(Error::Other(format!(
                        "drv parse: bad sequence at {}",
                        self.i
                    )))
                }
            }
        }
    }
}

fn as_str(t: &Term) -> Result<String> {
    match t {
        Term::Str(s) => Ok(s.clone()),
        _ => Err(Error::Other("drv parse: expected string".into())),
    }
}
fn as_list(t: &Term) -> Result<&Vec<Term>> {
    match t {
        Term::List(l) | Term::Tuple(l) => Ok(l),
        _ => Err(Error::Other("drv parse: expected list".into())),
    }
}

/// Parse the text of a `.drv` file into the same shape `nix derivation show` gives.
pub fn parse_drv(drv_path: &str, text: &str) -> Result<DrvInfo> {
    let text = text.trim_end();
    if !text.starts_with("Derive(") {
        return Err(Error::Other(format!("{drv_path}: not a Derive(...) term")));
    }
    let mut p = Parser {
        s: text.as_bytes(),
        i: "Derive".len(),
    };
    let fields = match p.term()? {
        Term::Tuple(f) => f,
        _ => return Err(Error::Other(format!("{drv_path}: malformed Derive term"))),
    };
    if fields.len() != 7 {
        return Err(Error::Other(format!(
            "{drv_path}: expected 7 fields, got {}",
            fields.len()
        )));
    }
    let mut d = DrvInfo {
        name: crate::iri::store_path_name(drv_path)
            .unwrap_or_default()
            .trim_end_matches(".drv")
            .to_string(),
        ..Default::default()
    };

    for o in as_list(&fields[0])? {
        let t = as_list(o)?;
        let name = as_str(&t[0])?;
        let path = as_str(&t[1])?;
        let algo = as_str(&t[2])?;
        let hash = as_str(&t[3])?;
        let (method, hash_algo) = split_algo(&algo);
        d.outputs.insert(
            name,
            OutputInfo {
                path: if path.is_empty() { None } else { Some(path) },
                hash_algo,
                method,
                hash: if hash.is_empty() { None } else { Some(hash) },
            },
        );
    }
    for i in as_list(&fields[1])? {
        let t = as_list(i)?;
        let path = as_str(&t[0])?;
        let outs: Vec<String> = as_list(&t[1])?.iter().map(as_str).collect::<Result<_>>()?;
        d.input_drvs.insert(
            path,
            InputDrv::Structured {
                outputs: outs,
                dynamic_outputs: serde_json::json!({}),
            },
        );
    }
    d.input_srcs = as_list(&fields[2])?
        .iter()
        .map(as_str)
        .collect::<Result<_>>()?;
    d.system = as_str(&fields[3])?;
    d.builder = as_str(&fields[4])?;
    d.args = as_list(&fields[5])?
        .iter()
        .map(as_str)
        .collect::<Result<_>>()?;
    for e in as_list(&fields[6])? {
        let t = as_list(e)?;
        d.env.insert(as_str(&t[0])?, as_str(&t[1])?);
    }
    Ok(d)
}

/// `"r:sha256"` → (Some("nar"), Some("sha256")); `"sha256"` → (Some("flat"), Some("sha256"));
/// `"text:sha256"` → (Some("text"), Some("sha256")); `""` → (None, None).
fn split_algo(a: &str) -> (Option<String>, Option<String>) {
    if a.is_empty() {
        return (None, None);
    }
    match a.split_once(':') {
        Some(("r", algo)) => (Some("nar".into()), Some(algo.into())),
        Some((m, algo)) => (Some(m.into()), Some(algo.into())),
        None => (Some("flat".into()), Some(a.into())),
    }
}

pub fn read_drv_file(path: &Path) -> Result<DrvInfo> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    parse_drv(&path.to_string_lossy(), &text)
}

/// Read a closure of .drv files starting from roots, following inputDrvs.
pub fn read_closure(roots: &[String]) -> Result<BTreeMap<String, DrvInfo>> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<String> = roots.to_vec();
    while let Some(p) = stack.pop() {
        if out.contains_key(&p) {
            continue;
        }
        let d = read_drv_file(Path::new(&p))?;
        for dep in d.input_drvs.keys() {
            if !out.contains_key(dep) {
                stack.push(dep.clone());
            }
        }
        out.insert(p, d);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_drv() {
        let text = r#"Derive([("out","/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-x","","")],[("/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-y.drv",["out"])],["/nix/store/cccccccccccccccccccccccccccccccc-s"],"x86_64-linux","/bin/sh",["-c","echo \"hi\\n\""],[("name","x"),("out","/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-x")])"#;
        let d = parse_drv("/nix/store/dddddddddddddddddddddddddddddddd-x.drv", text).unwrap();
        assert_eq!(d.name, "x");
        assert_eq!(d.system, "x86_64-linux");
        assert_eq!(d.args, vec!["-c", "echo \"hi\\n\""]);
        assert_eq!(
            d.outputs["out"].path.as_deref(),
            Some("/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-x")
        );
        assert_eq!(d.input_drvs.len(), 1);
        assert_eq!(d.env["name"], "x");
    }

    #[test]
    fn parses_ca_output() {
        let text = r#"Derive([("out","","r:sha256","")],[],[],"x86_64-linux","/bin/sh",[],[])"#;
        let d = parse_drv("/nix/store/dddddddddddddddddddddddddddddddd-x.drv", text).unwrap();
        assert!(d.is_content_addressed());
        assert_eq!(d.outputs["out"].method.as_deref(), Some("nar"));
    }
}
