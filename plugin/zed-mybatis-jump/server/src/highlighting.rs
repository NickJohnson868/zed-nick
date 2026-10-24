use crate::sql_text;
use quick_xml::{Reader, events::Event};
use serde_json::{Value, json};
use sqlparser::{
    dialect::PostgreSqlDialect,
    keywords::Keyword,
    tokenizer::{Token, Tokenizer, Whitespace},
};

pub const TYPES: [&str; 5] = [
    "mybatisKeyword",
    "mybatisIdentifier",
    "mybatisString",
    "mybatisNumber",
    "mybatisComment",
];

// Color XML text only; dynamic tags and their attributes remain XML.
pub fn tokens(xml: &str) -> Value {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut mapper = false;
    let mut sql_depth = None;
    let mut spans = Vec::new();
    loop {
        let offset = reader.buffer_position() as usize;
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => break,
        };
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    mapper = tag.name().as_ref() == b"mapper";
                }
                if mapper
                    && depth == 1
                    && matches!(
                        tag.name().as_ref(),
                        b"select" | b"insert" | b"update" | b"delete" | b"sql"
                    )
                {
                    sql_depth = Some(depth);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if sql_depth == Some(depth) {
                    sql_depth = None;
                }
            }
            Event::Text(text) if sql_depth.is_some() => collect(
                xml,
                std::str::from_utf8(&text).unwrap_or(""),
                offset,
                &mut spans,
            ),
            Event::CData(text) if sql_depth.is_some() => collect(
                xml,
                std::str::from_utf8(&text).unwrap_or(""),
                offset + 9,
                &mut spans,
            ),
            Event::Eof => break,
            _ => {}
        }
    }
    spans.sort_unstable();
    let mut data = Vec::new();
    let (mut line, mut column) = (0, 0);
    for (next_line, next_column, length, kind) in spans {
        data.extend([
            next_line - line,
            if next_line == line {
                next_column - column
            } else {
                next_column
            },
            length,
            kind,
            0,
        ]);
        line = next_line;
        column = next_column;
    }
    json!({"data":data})
}

fn collect(xml: &str, text: &str, base: usize, spans: &mut Vec<(u32, u32, u32, u32)>) {
    let mut tokens = Vec::new();
    // Keep tokens preceding an incomplete string while the user is typing.
    let _ =
        Tokenizer::new(&PostgreSqlDialect {}, text).tokenize_with_location_into_buf(&mut tokens);
    for token in tokens {
        let kind = match token.token {
            Token::Word(word) => {
                if word.quote_style.is_none()
                    && matches!(
                        word.keyword,
                        Keyword::SELECT
                            | Keyword::FROM
                            | Keyword::WHERE
                            | Keyword::JOIN
                            | Keyword::LEFT
                            | Keyword::RIGHT
                            | Keyword::FULL
                            | Keyword::INNER
                            | Keyword::OUTER
                            | Keyword::CROSS
                            | Keyword::ON
                            | Keyword::AS
                            | Keyword::AND
                            | Keyword::OR
                            | Keyword::NOT
                            | Keyword::NULL
                            | Keyword::IS
                            | Keyword::IN
                            | Keyword::EXISTS
                            | Keyword::BETWEEN
                            | Keyword::LIKE
                            | Keyword::ILIKE
                            | Keyword::DISTINCT
                            | Keyword::ORDER
                            | Keyword::BY
                            | Keyword::GROUP
                            | Keyword::HAVING
                            | Keyword::LIMIT
                            | Keyword::OFFSET
                            | Keyword::INSERT
                            | Keyword::INTO
                            | Keyword::VALUES
                            | Keyword::UPDATE
                            | Keyword::SET
                            | Keyword::DELETE
                            | Keyword::RETURNING
                            | Keyword::WITH
                            | Keyword::RECURSIVE
                            | Keyword::UNION
                            | Keyword::ALL
                            | Keyword::INTERSECT
                            | Keyword::EXCEPT
                            | Keyword::CASE
                            | Keyword::WHEN
                            | Keyword::THEN
                            | Keyword::ELSE
                            | Keyword::END
                            | Keyword::ASC
                            | Keyword::DESC
                            | Keyword::TRUE
                            | Keyword::FALSE
                            | Keyword::OVER
                            | Keyword::PARTITION
                            | Keyword::FILTER
                            | Keyword::CONFLICT
                            | Keyword::DO
                            | Keyword::NOTHING
                            | Keyword::FETCH
                            | Keyword::FIRST
                            | Keyword::ROWS
                            | Keyword::ONLY
                            | Keyword::USING
                    )
                {
                    0
                } else {
                    1
                }
            }
            Token::SingleQuotedString(_) | Token::DollarQuotedString(_) => 2,
            Token::Number(_, _) => 3,
            Token::Whitespace(
                Whitespace::SingleLineComment { .. } | Whitespace::MultiLineComment(_),
            ) => 4,
            _ => continue,
        };
        let start = byte(text, token.span.start.line, token.span.start.column);
        let end = byte(text, token.span.end.line, token.span.end.column);
        if start >= end {
            continue;
        }
        // Emit one token per line as required by LSP clients.
        let mut cursor = start;
        for segment in text[start..end].split_inclusive('\n') {
            let content = segment.trim_end_matches(['\r', '\n']);
            if !content.is_empty() {
                let position = sql_text::position(xml, base + cursor);
                spans.push((
                    position["line"].as_u64().unwrap() as u32,
                    position["character"].as_u64().unwrap() as u32,
                    content.encode_utf16().count() as u32,
                    kind,
                ));
            }
            cursor += segment.len();
        }
    }
}

fn byte(text: &str, line: u64, column: u64) -> usize {
    let mut offset = 0;
    for (index, part) in text.split_inclusive('\n').enumerate() {
        if index as u64 + 1 == line {
            return offset
                + part
                    .char_indices()
                    .nth(column.saturating_sub(1) as usize)
                    .map_or(part.len(), |(i, _)| i);
        }
        offset += part.len();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_sql_and_dynamic_text_without_touching_xml_or_java() {
        let xml = "<mapper namespace='X'><select id='x'>SELECT name FROM o2o.shop <if test='x'>WHERE id = 1</if></select></mapper>";
        let result = tokens(xml);
        let data = result["data"].as_array().unwrap();
        assert!(data.chunks(5).any(|t| t[3] == 0));
        assert!(data.chunks(5).any(|t| t[3] == 1));
        assert_eq!(tokens("<project>SELECT name</project>")["data"], json!([]));
        assert_eq!(tokens("public interface Mapper {}")["data"], json!([]));
    }
}
