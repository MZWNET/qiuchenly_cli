use anyhow::{bail, Result};

pub(super) fn matches(
    condition: &str,
    language: &str,
    architecture: &str,
    os_version: &str,
) -> Result<bool> {
    if condition.trim().is_empty() {
        return Ok(true);
    }
    let tokens = tokenize(condition)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        condition,
    };
    let value = parser.parse_or(language, architecture, os_version)?;
    if parser.pos != parser.tokens.len() {
        bail!("条件表达式存在多余内容: {condition}");
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Var(String),
    Lit(String),
    LParen,
    RParen,
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
    And,
    Or,
}

fn tokenize(condition: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = condition.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let token = match c {
            '[' => {
                let Some(len) = chars[i + 1..].iter().position(|&c| c == ']') else {
                    bail!("条件表达式存在未闭合的变量: {condition}");
                };
                let name: String = chars[i + 1..i + 1 + len].iter().collect();
                if name.is_empty() {
                    bail!("条件表达式存在空变量: {condition}");
                }
                i += len + 1;
                Token::Var(name)
            }
            '(' => Token::LParen,
            ')' => Token::RParen,
            '=' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 1;
                    Token::Eq
                } else {
                    bail!("条件表达式语法错误: {condition}");
                }
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 1;
                    Token::Ne
                } else {
                    bail!("条件表达式语法错误: {condition}");
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 1;
                    Token::Ge
                } else {
                    Token::Gt
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    i += 1;
                    Token::Le
                } else {
                    Token::Lt
                }
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    i += 1;
                    Token::And
                } else {
                    bail!("条件表达式语法错误: {condition}");
                }
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    i += 1;
                    Token::Or
                } else {
                    bail!("条件表达式语法错误: {condition}");
                }
            }
            _ => {
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && !matches!(
                        chars[i],
                        '[' | ']' | '(' | ')' | '=' | '!' | '>' | '<' | '&' | '|'
                    )
                {
                    i += 1;
                }
                if i == start {
                    bail!("条件表达式语法错误: {condition}");
                }
                tokens.push(Token::Lit(chars[start..i].iter().collect()));
                continue;
            }
        };
        tokens.push(token);
        i += 1;
    }
    if tokens.is_empty() {
        bail!("条件表达式为空: {condition}");
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    condition: &'a str,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        Some(token)
    }

    fn parse_or(&mut self, language: &str, architecture: &str, os_version: &str) -> Result<bool> {
        let mut value = self.parse_and(language, architecture, os_version)?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.pos += 1;
            let right = self.parse_and(language, architecture, os_version)?;
            value = value || right;
        }
        Ok(value)
    }

    fn parse_and(&mut self, language: &str, architecture: &str, os_version: &str) -> Result<bool> {
        let mut value = self.parse_primary(language, architecture, os_version)?;
        while matches!(self.peek(), Some(Token::And)) {
            self.pos += 1;
            let right = self.parse_primary(language, architecture, os_version)?;
            value = value && right;
        }
        Ok(value)
    }

    fn parse_primary(
        &mut self,
        language: &str,
        architecture: &str,
        os_version: &str,
    ) -> Result<bool> {
        match self.advance() {
            Some(Token::LParen) => {
                let value = self.parse_or(language, architecture, os_version)?;
                if !matches!(self.advance(), Some(Token::RParen)) {
                    bail!("条件表达式缺少右括号: {}", self.condition);
                }
                Ok(value)
            }
            Some(Token::Var(name)) => {
                self.parse_compare(self.value_of(&name, language, architecture, os_version)?)
            }
            Some(Token::Lit(value)) => self.parse_compare(value),
            _ => bail!("条件表达式语法错误: {}", self.condition),
        }
    }

    fn parse_compare(&mut self, lhs: String) -> Result<bool> {
        let operator = match self.advance() {
            Some(
                operator @ (Token::Eq | Token::Ne | Token::Ge | Token::Gt | Token::Le | Token::Lt),
            ) => operator,
            _ => bail!("条件表达式缺少比较运算符: {}", self.condition),
        };
        let token = self
            .advance()
            .ok_or_else(|| anyhow::anyhow!("条件表达式缺少比较值: {}", self.condition))?;
        let rhs = match token {
            Token::Var(name) => bail!("条件表达式右侧不能是变量 [{name}]: {}", self.condition),
            Token::Lit(value) => value,
            _ => bail!("条件表达式比较值无效: {}", self.condition),
        };
        match operator {
            Token::Eq => Ok(lhs == rhs),
            Token::Ne => Ok(lhs != rhs),
            operator => {
                let ordering = version_compare::compare(&lhs, &rhs)
                    .map_err(|_| anyhow::anyhow!("无法比较版本: {lhs} 和 {rhs}"))?;
                Ok(match operator {
                    Token::Ge => matches!(
                        ordering,
                        version_compare::Cmp::Gt | version_compare::Cmp::Eq
                    ),
                    Token::Gt => ordering == version_compare::Cmp::Gt,
                    Token::Le => matches!(
                        ordering,
                        version_compare::Cmp::Lt | version_compare::Cmp::Eq
                    ),
                    Token::Lt => ordering == version_compare::Cmp::Lt,
                    _ => unreachable!(),
                })
            }
        }
    }

    fn value_of(
        &self,
        name: &str,
        language: &str,
        architecture: &str,
        os_version: &str,
    ) -> Result<String> {
        match name {
            "installLanguage" => Ok(language.to_owned()),
            "OSArchitecture" => Ok(architecture.to_owned()),
            "OSVersion" => Ok(os_version.to_owned()),
            _ => bail!("条件表达式包含未知变量 [{name}]: {}", self.condition),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE_LANGUAGE: &str = "[installLanguage]==ja_JP||[installLanguage]==ko_KR||[installLanguage]==zh_CN||[installLanguage]==zh_TW||[installLanguage]==en_US||[installLanguage]==fr_CA";

    #[test]
    fn evaluates_real_language_set_condition() {
        assert!(matches(CORE_LANGUAGE, "zh_CN", "arm64", "14.0").unwrap());
        assert!(matches(CORE_LANGUAGE, "fr_CA", "x64", "14.0").unwrap());
        assert!(!matches(CORE_LANGUAGE, "de_DE", "arm64", "14.0").unwrap());
    }

    #[test]
    fn evaluates_architecture_condition() {
        assert!(matches("[OSArchitecture]==arm64", "en_US", "arm64", "14.0").unwrap());
        assert!(!matches("[OSArchitecture]==arm64", "en_US", "x64", "14.0").unwrap());
        assert!(matches("[OSArchitecture]!=arm64", "en_US", "x64", "14.0").unwrap());
    }

    #[test]
    fn evaluates_composite_expression_with_parentheses() {
        let condition =
            "([installLanguage]==en_US||[installLanguage]==en_GB)&&[OSArchitecture]==arm64";
        assert!(matches(condition, "en_US", "arm64", "14.0").unwrap());
        assert!(!matches(condition, "en_US", "x64", "14.0").unwrap());
        assert!(!matches(condition, "de_DE", "arm64", "14.0").unwrap());
    }

    #[test]
    fn empty_condition_matches() {
        assert!(matches("", "en_US", "arm64", "14.0").unwrap());
        assert!(matches("   ", "en_US", "arm64", "14.0").unwrap());
    }

    #[test]
    fn rejects_unknown_variable() {
        assert!(matches("[installLang]==en_US", "en_US", "arm64", "14.0").is_err());
        assert!(matches("[osarchitecture]==arm64", "en_US", "arm64", "14.0").is_err());
    }

    #[test]
    fn rejects_invalid_syntax() {
        assert!(matches("[installLanguage]==", "en_US", "arm64", "14.0").is_err());
        assert!(matches("==en_US", "en_US", "arm64", "14.0").is_err());
        assert!(matches("[installLanguage]", "en_US", "arm64", "14.0").is_err());
        assert!(matches("[installLanguage]==en_US&&", "en_US", "arm64", "14.0").is_err());
        assert!(matches("(a==b", "en_US", "arm64", "14.0").is_err());
        assert!(matches("[installLanguage]==en_US en_US", "en_US", "arm64", "14.0").is_err());
    }

    #[test]
    fn evaluates_os_version_comparisons() {
        for (condition, version, expected) in [
            ("[OSVersion]>=14.0", "14.0", true),
            ("[OSVersion]>=14.0", "13.9", false),
            ("[OSVersion]>14.0", "14.0", false),
            ("[OSVersion]>14.0", "14.1", true),
            ("[OSVersion]<=14.0", "14.0", true),
            ("[OSVersion]<=14.0", "14.1", false),
            ("[OSVersion]<14.0", "14.0", false),
            ("[OSVersion]<14.0", "13.9", true),
            ("[OSVersion]==14.0", "14.0.0", false),
            ("[OSVersion]!=14.0", "14.0.0", true),
            (
                "([OSVersion]>=14.0&&[OSArchitecture]==arm64)||[installLanguage]==ja_JP",
                "14.2",
                true,
            ),
        ] {
            assert_eq!(
                matches(condition, "en_US", "arm64", version).unwrap(),
                expected,
                "{condition} with {version}"
            );
        }
    }

    #[test]
    fn rejects_invalid_version_comparisons() {
        for condition in [
            "[OSVersion]>=",
            "[OSVersion]=>14.0",
            "[OSVersion]><14.0",
            "[OSVersion]>=invalid",
        ] {
            assert!(
                matches(condition, "en_US", "arm64", "14.0").is_err(),
                "{condition}"
            );
        }
        assert!(matches("[OSVersion]>=14.0", "en_US", "arm64", "invalid").is_err());
    }
}
