//! Pure-Rust recursive-descent arithmetic parser and evaluator.
//! Strictly zero OS, shell, or process execution.

use std::f64::consts::{E, PI};

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
}

const MAX_CALC_INPUT_BYTES: usize = 8_192;
const MAX_CALC_TOKENS: usize = 512;

pub struct Lexer<'a> {
    input_len: usize,
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input_len: input.len(),
            chars: input.chars().peekable(),
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        if self.input_len > MAX_CALC_INPUT_BYTES {
            return Err(format!(
                "Arithmetic expression exceeds maximum input size ({} > {} bytes)",
                self.input_len, MAX_CALC_INPUT_BYTES
            ));
        }

        let mut tokens = Vec::new();

        while let Some(&ch) = self.chars.peek() {
            if tokens.len() >= MAX_CALC_TOKENS {
                return Err(format!(
                    "Arithmetic expression exceeds maximum token count ({})",
                    MAX_CALC_TOKENS
                ));
            }
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.chars.next();
                }
                '+' => {
                    self.chars.next();
                    tokens.push(Token::Plus);
                }
                '-' => {
                    self.chars.next();
                    tokens.push(Token::Minus);
                }
                '*' => {
                    self.chars.next();
                    tokens.push(Token::Star);
                }
                '/' => {
                    self.chars.next();
                    tokens.push(Token::Slash);
                }
                '%' => {
                    self.chars.next();
                    tokens.push(Token::Percent);
                }
                '^' => {
                    self.chars.next();
                    tokens.push(Token::Caret);
                }
                '(' => {
                    self.chars.next();
                    tokens.push(Token::LParen);
                }
                ')' => {
                    self.chars.next();
                    tokens.push(Token::RParen);
                }
                ',' => {
                    self.chars.next();
                    tokens.push(Token::Comma);
                }
                '0'..='9' | '.' => {
                    tokens.push(self.read_number()?);
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    tokens.push(self.read_ident()?);
                }
                illegal => {
                    return Err(format!(
                        "Illegal character '{}' in arithmetic expression",
                        illegal
                    ));
                }
            }
        }

        Ok(tokens)
    }

    fn read_number(&mut self) -> Result<Token, String> {
        let mut num_str = String::new();
        let mut has_dot = false;

        while let Some(&ch) = self.chars.peek() {
            if ch.is_ascii_digit() {
                num_str.push(ch);
                self.chars.next();
            } else if ch == '.' {
                if has_dot {
                    return Err("Multiple decimal points in number".into());
                }
                has_dot = true;
                num_str.push(ch);
                self.chars.next();
            } else if ch == 'e' || ch == 'E' {
                num_str.push(ch);
                self.chars.next();
                if let Some(&sign) = self.chars.peek() {
                    if sign == '+' || sign == '-' {
                        num_str.push(sign);
                        self.chars.next();
                    }
                }
            } else {
                break;
            }
        }

        let val = num_str
            .parse::<f64>()
            .map_err(|e| format!("Invalid numeric literal '{}': {}", num_str, e))?;
        Ok(Token::Number(val))
    }

    fn read_ident(&mut self) -> Result<Token, String> {
        let mut ident = String::new();
        while let Some(&ch) = self.chars.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ident.push(ch);
                self.chars.next();
            } else {
                break;
            }
        }
        Ok(Token::Ident(ident.to_ascii_lowercase()))
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    max_depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
            max_depth: 32,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    pub fn parse(&mut self) -> Result<f64, String> {
        if self.tokens.is_empty() {
            return Err("Empty expression".into());
        }
        let val = self.parse_additive()?;
        if self.pos < self.tokens.len() {
            return Err(format!(
                "Unexpected trailing token: {:?}",
                self.tokens[self.pos]
            ));
        }
        if val.is_nan() || val.is_infinite() {
            return Err("Arithmetic overflow or undefined result (NaN/Inf)".into());
        }
        Ok(val)
    }

    fn enter_depth(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > self.max_depth {
            Err("Expression exceeds maximum recursion depth limit (32)".into())
        } else {
            Ok(())
        }
    }

    fn leave_depth(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        }
    }

    fn parse_additive(&mut self) -> Result<f64, String> {
        self.enter_depth()?;
        let mut left = self.parse_multiplicative()?;

        while let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    left += right;
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    left -= right;
                }
                _ => break,
            }
        }
        self.leave_depth();
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<f64, String> {
        self.enter_depth()?;
        let mut left = self.parse_power()?;

        while let Some(tok) = self.peek() {
            match tok {
                Token::Star => {
                    self.advance();
                    let right = self.parse_power()?;
                    left *= right;
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right.abs() < 1e-15 {
                        return Err("Division by zero".into());
                    }
                    left /= right;
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right.abs() < 1e-15 {
                        return Err("Division by zero in modulo operation".into());
                    }
                    left %= right;
                }
                _ => break,
            }
        }
        self.leave_depth();
        Ok(left)
    }

    fn parse_power(&mut self) -> Result<f64, String> {
        self.enter_depth()?;
        let base = self.parse_unary()?;

        if let Some(Token::Caret) = self.peek() {
            self.advance();
            // Right-associative exponentiation: 2 ^ 3 ^ 2 == 2 ^ (3 ^ 2)
            let exponent = self.parse_power()?;
            let res = base.powf(exponent);
            if res.is_nan() || res.is_infinite() {
                return Err("Exponentiation resulted in non-finite value or complex result".into());
            }
            self.leave_depth();
            return Ok(res);
        }

        self.leave_depth();
        Ok(base)
    }

    fn parse_unary(&mut self) -> Result<f64, String> {
        self.enter_depth()?;
        if let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.advance();
                    let val = self.parse_unary()?;
                    self.leave_depth();
                    return Ok(val);
                }
                Token::Minus => {
                    self.advance();
                    let val = self.parse_unary()?;
                    self.leave_depth();
                    return Ok(-val);
                }
                _ => {}
            }
        }

        let val = self.parse_primary()?;
        self.leave_depth();
        Ok(val)
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        self.enter_depth()?;
        match self.advance() {
            Some(Token::Number(n)) => {
                self.leave_depth();
                Ok(n)
            }
            Some(Token::LParen) => {
                let val = self.parse_additive()?;
                match self.advance() {
                    Some(Token::RParen) => {
                        self.leave_depth();
                        Ok(val)
                    }
                    _ => Err("Missing closing parenthesis ')'".into()),
                }
            }
            Some(Token::Ident(name)) => {
                if let Some(Token::LParen) = self.peek() {
                    self.advance(); // consume '('
                    let mut args = Vec::new();
                    if let Some(Token::RParen) = self.peek() {
                        self.advance(); // empty args
                    } else {
                        loop {
                            let arg = self.parse_additive()?;
                            args.push(arg);
                            match self.peek() {
                                Some(Token::Comma) => {
                                    self.advance();
                                }
                                Some(Token::RParen) => {
                                    self.advance();
                                    break;
                                }
                                _ => return Err("Expected ',' or ')' in function call".into()),
                            }
                        }
                    }

                    let res = self.evaluate_function(&name, &args)?;
                    self.leave_depth();
                    Ok(res)
                } else {
                    let res = match name.as_str() {
                        "pi" => PI,
                        "e" => E,
                        other => {
                            return Err(format!(
                                "Unknown constant or function '{}'. Shell commands are prohibited.",
                                other
                            ))
                        }
                    };
                    self.leave_depth();
                    Ok(res)
                }
            }
            Some(tok) => Err(format!("Unexpected token in expression: {:?}", tok)),
            None => Err("Unexpected end of expression".into()),
        }
    }

    fn evaluate_function(&self, name: &str, args: &[f64]) -> Result<f64, String> {
        match name {
            "sqrt" => {
                if args.len() != 1 {
                    return Err("sqrt() requires exactly 1 argument".into());
                }
                if args[0] < 0.0 {
                    return Err("Cannot take sqrt of negative number".into());
                }
                Ok(args[0].sqrt())
            }
            "abs" => {
                if args.len() != 1 {
                    return Err("abs() requires exactly 1 argument".into());
                }
                Ok(args[0].abs())
            }
            "sin" => {
                if args.len() != 1 {
                    return Err("sin() requires exactly 1 argument".into());
                }
                Ok(args[0].sin())
            }
            "cos" => {
                if args.len() != 1 {
                    return Err("cos() requires exactly 1 argument".into());
                }
                Ok(args[0].cos())
            }
            "tan" => {
                if args.len() != 1 {
                    return Err("tan() requires exactly 1 argument".into());
                }
                Ok(args[0].tan())
            }
            "ln" => {
                if args.len() != 1 {
                    return Err("ln() requires exactly 1 argument".into());
                }
                if args[0] <= 0.0 {
                    return Err("ln() argument must be strictly positive".into());
                }
                Ok(args[0].ln())
            }
            "log" | "log10" => {
                if args.len() == 1 {
                    if args[0] <= 0.0 {
                        return Err("log10() argument must be strictly positive".into());
                    }
                    Ok(args[0].log10())
                } else if args.len() == 2 {
                    if args[0] <= 0.0 || args[1] <= 0.0 || (args[1] - 1.0).abs() < 1e-15 {
                        return Err("Invalid domain or base in log(x, base)".into());
                    }
                    Ok(args[0].log(args[1]))
                } else {
                    Err("log() requires 1 or 2 arguments".into())
                }
            }
            "exp" => {
                if args.len() != 1 {
                    return Err("exp() requires exactly 1 argument".into());
                }
                Ok(args[0].exp())
            }
            "round" => {
                if args.len() != 1 {
                    return Err("round() requires exactly 1 argument".into());
                }
                Ok(args[0].round())
            }
            "floor" => {
                if args.len() != 1 {
                    return Err("floor() requires exactly 1 argument".into());
                }
                Ok(args[0].floor())
            }
            "ceil" => {
                if args.len() != 1 {
                    return Err("ceil() requires exactly 1 argument".into());
                }
                Ok(args[0].ceil())
            }
            "min" => {
                if args.len() != 2 {
                    return Err("min() requires exactly 2 arguments".into());
                }
                Ok(args[0].min(args[1]))
            }
            "max" => {
                if args.len() != 2 {
                    return Err("max() requires exactly 2 arguments".into());
                }
                Ok(args[0].max(args[1]))
            }
            other => Err(format!(
                "Unsupported math function '{}'. OS execution is prohibited.",
                other
            )),
        }
    }
}

/// Evaluates a mathematical expression safely with strict recursion and size limits.
pub fn evaluate_expression(input: &str) -> Result<f64, String> {
    if input.len() > 1024 {
        return Err("Expression exceeds maximum permitted limit of 1024 characters".into());
    }

    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_arithmetic() {
        assert_eq!(evaluate_expression("2 + 2").unwrap(), 4.0);
        assert_eq!(evaluate_expression("10 - 4").unwrap(), 6.0);
        assert_eq!(evaluate_expression("3 * 5").unwrap(), 15.0);
        assert_eq!(evaluate_expression("20 / 4").unwrap(), 5.0);
        assert_eq!(evaluate_expression("14 % 4").unwrap(), 2.0);
    }

    #[test]
    fn test_precedence_and_grouping() {
        assert_eq!(evaluate_expression("2 + 3 * 4").unwrap(), 14.0);
        assert_eq!(evaluate_expression("(2 + 3) * 4").unwrap(), 20.0);
        assert_eq!(evaluate_expression("2 ^ 3 ^ 2").unwrap(), 512.0); // 2^(3^2) = 2^9 = 512
        assert_eq!(evaluate_expression("(2 ^ 3) ^ 2").unwrap(), 64.0);
    }

    #[test]
    fn test_unary_operators() {
        assert_eq!(evaluate_expression("-5 + 10").unwrap(), 5.0);
        assert_eq!(evaluate_expression("-(3 * 4)").unwrap(), -12.0);
        assert_eq!(evaluate_expression("+7 - -3").unwrap(), 10.0);
    }

    #[test]
    fn test_functions_and_constants() {
        assert_eq!(evaluate_expression("sqrt(144)").unwrap(), 12.0);
        assert_eq!(evaluate_expression("abs(-42.5)").unwrap(), 42.5);
        assert_eq!(evaluate_expression("min(10, 25)").unwrap(), 10.0);
        assert_eq!(evaluate_expression("max(10, 25)").unwrap(), 25.0);
        assert!((evaluate_expression("sin(0)").unwrap() - 0.0).abs() < 1e-10);
        assert!((evaluate_expression("cos(0)").unwrap() - 1.0).abs() < 1e-10);
        assert!((evaluate_expression("pi").unwrap() - PI).abs() < 1e-10);
        assert!((evaluate_expression("ln(e)").unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_division_by_zero_and_domain() {
        assert!(evaluate_expression("5 / 0").is_err());
        assert!(evaluate_expression("5 % 0").is_err());
        assert!(evaluate_expression("sqrt(-4)").is_err());
        assert!(evaluate_expression("ln(-1)").is_err());
    }

    #[test]
    fn test_malicious_injection_attempts() {
        assert!(evaluate_expression("powershell -Command Get-Process").is_err());
        assert!(evaluate_expression("[System.Diagnostics.Process]::Start('calc')").is_err());
        assert!(evaluate_expression("rm -rf /").is_err());
        assert!(evaluate_expression("cat /etc/passwd").is_err());
        assert!(evaluate_expression("; calc.exe").is_err());
        assert!(evaluate_expression("whoami").is_err());
        assert!(evaluate_expression("$env:PATH").is_err());
    }

    #[test]
    fn test_recursion_depth_limit() {
        let deep = "(".repeat(40) + "1" + &")".repeat(40);
        assert!(evaluate_expression(&deep).is_err());
    }

    #[test]
    fn test_lexer_input_and_token_limits() {
        let huge_input = "1+".repeat(5000) + "1";
        assert!(evaluate_expression(&huge_input).is_err());

        let many_tokens = "1+".repeat(300) + "1"; // > 512 tokens
        assert!(evaluate_expression(&many_tokens).is_err());
    }
}
