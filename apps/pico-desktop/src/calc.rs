/// Lightweight zero-dependency inline calculator for instant Quick Calc previews inside the companion.
pub fn try_eval_quick_math(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    // Require at least one mathematical operator or parens so plain text isn't echoed as a number.
    let has_op = trimmed.contains('+')
        || trimmed.contains('-')
        || trimmed.contains('*')
        || trimmed.contains('/')
        || trimmed.contains('%')
        || trimmed.contains('^')
        || trimmed.to_lowercase().contains("sqrt(");
    if !has_op {
        return None;
    }
    let val = eval_expr(trimmed)?;
    if !val.is_finite() {
        return None;
    }
    if (val - val.round()).abs() < 1e-9 && val.abs() < 1e15 {
        Some(format!("{}", val.round() as i64))
    } else {
        let formatted = format!("{:.6}", val);
        Some(formatted.trim_end_matches('0').trim_end_matches('.').to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    LParen,
    RParen,
    Sqrt,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b.is_ascii_digit() || b == b'.' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let num: f64 = s[start..i].parse().ok()?;
            out.push(Tok::Num(num));
            continue;
        }
        if b.is_ascii_alphabetic() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let ident = s[start..i].to_ascii_lowercase();
            match ident.as_str() {
                "sqrt" => out.push(Tok::Sqrt),
                "pi" => out.push(Tok::Num(std::f64::consts::PI)),
                _ => return None,
            }
            continue;
        }
        match b {
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' => out.push(Tok::Op(b as char)),
            b'(' => out.push(Tok::LParen),
            b')' => out.push(Tok::RParen),
            _ => return None,
        }
        i += 1;
    }
    Some(out)
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_add_sub(&mut self, depth: usize) -> Option<f64> {
        let mut lhs = self.parse_mul_div(depth)?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek().cloned() {
            self.next();
            let rhs = self.parse_mul_div(depth)?;
            lhs = if c == '+' { lhs + rhs } else { lhs - rhs };
        }
        Some(lhs)
    }

    fn parse_mul_div(&mut self, depth: usize) -> Option<f64> {
        let mut lhs = self.parse_pow(depth)?;
        while let Some(Tok::Op(c @ ('*' | '/' | '%'))) = self.peek().cloned() {
            self.next();
            let rhs = self.parse_pow(depth)?;
            match c {
                '*' => lhs *= rhs,
                '/' => {
                    if rhs.abs() < 1e-12 {
                        return None;
                    }
                    lhs /= rhs;
                }
                '%' => {
                    if rhs.abs() < 1e-12 {
                        return None;
                    }
                    lhs %= rhs;
                }
                _ => unreachable!(),
            }
        }
        Some(lhs)
    }

    fn parse_pow(&mut self, depth: usize) -> Option<f64> {
        let base = self.parse_unary(depth)?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.next();
            let exp = self.parse_pow(depth + 1)?;
            Some(base.powf(exp))
        } else {
            Some(base)
        }
    }

    fn parse_unary(&mut self, depth: usize) -> Option<f64> {
        if depth > 48 {
            return None;
        }
        match self.peek().cloned() {
            Some(Tok::Op('-')) => {
                self.next();
                Some(-self.parse_unary(depth + 1)?)
            }
            Some(Tok::Op('+')) => {
                self.next();
                self.parse_unary(depth + 1)
            }
            Some(Tok::Sqrt) => {
                self.next();
                let arg = self.parse_primary(depth + 1)?;
                if arg < 0.0 {
                    None
                } else {
                    Some(arg.sqrt())
                }
            }
            _ => self.parse_primary(depth + 1),
        }
    }

    fn parse_primary(&mut self, depth: usize) -> Option<f64> {
        if depth > 48 {
            return None;
        }
        match self.next()?.clone() {
            Tok::Num(n) => Some(n),
            Tok::LParen => {
                let inner = self.parse_add_sub(depth + 1)?;
                match self.next() {
                    Some(Tok::RParen) => Some(inner),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn eval_expr(expr: &str) -> Option<f64> {
    let toks = tokenize(expr)?;
    let mut p = Parser { toks: &toks, pos: 0 };
    let val = p.parse_add_sub(0)?;
    if p.pos == toks.len() {
        Some(val)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quick_calc_safety_and_accuracy() {
        assert_eq!(try_eval_quick_math("25 * 4 + 12 / 3"), Some("104".into()));
        assert_eq!(try_eval_quick_math("sqrt(81) + 2^4"), Some("25".into()));
        assert_eq!(try_eval_quick_math("5 / 0"), None);
        assert_eq!(try_eval_quick_math("open notepad"), None);
    }
}
