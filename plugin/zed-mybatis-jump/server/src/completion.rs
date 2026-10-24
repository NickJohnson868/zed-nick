use crate::{database::Catalog, sql_text};
use serde_json::{Value, json};
use sqlparser::{
    dialect::PostgreSqlDialect,
    tokenizer::{Token, Tokenizer},
};
use std::collections::BTreeMap;

fn word(token: &Token) -> Option<String> {
    if let Token::Word(word) = token {
        Some(if word.quote_style.is_some() {
            word.value.clone()
        } else {
            word.value.to_lowercase()
        })
    } else {
        None
    }
}

fn tokens(sql: &str) -> Vec<Token> {
    Tokenizer::new(&PostgreSqlDialect {}, sql)
        .tokenize()
        .unwrap_or_default()
        .into_iter()
        .filter(|token| !matches!(token, Token::Whitespace(_)))
        .collect()
}

fn relations(tokens: &[Token], catalog: &Catalog) -> BTreeMap<String, (String, String)> {
    let mut result = BTreeMap::new();
    for (index, token) in tokens.iter().enumerate() {
        if !word(token).is_some_and(|s| matches!(s.as_str(), "from" | "join" | "update" | "into")) {
            continue;
        }
        let Some(first) = tokens.get(index + 1).and_then(word) else {
            continue;
        };
        let mut end = index + 2;
        let (schema, table) = if tokens.get(end) == Some(&Token::Period) {
            let Some(table) = tokens.get(end + 1).and_then(word) else {
                continue;
            };
            end += 2;
            (first, table)
        } else {
            let Some(schema) = catalog.search_path.iter().find(|schema| {
                catalog
                    .tables
                    .contains_key(&(schema.to_string(), first.clone()))
            }) else {
                continue;
            };
            (schema.clone(), first)
        };
        if !catalog
            .tables
            .contains_key(&(schema.clone(), table.clone()))
        {
            continue;
        }
        let mut alias = table.clone();
        if tokens.get(end).and_then(word).as_deref() == Some("as") {
            end += 1;
        }
        if let Some(Token::Word(candidate)) = tokens.get(end)
            && candidate.keyword == sqlparser::keywords::Keyword::NoKeyword
        {
            alias = word(&tokens[end]).unwrap();
        }
        result.insert(alias, (schema, table));
    }
    result
}

fn quoted(name: &str) -> String {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
    {
        name.into()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

pub fn complete(xml: &str, offset: usize, catalog: &Catalog) -> Vec<Value> {
    let prefix = &xml[..offset];
    if let Some(open) = prefix.rfind("#{")
        && prefix.rfind('}').is_none_or(|close| close < open)
    {
        return Vec::new();
    }
    let Some(block) = sql_text::blocks(xml)
        .into_iter()
        .find(|block| !block.dynamic && block.start <= offset && offset <= block.end)
    else {
        return Vec::new();
    };
    let sql_offset = block.sql_offset(offset).min(block.sql.len());
    let before = &block.sql[..sql_offset];
    let prefix_start = before
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_alphanumeric() && *ch != '_')
        .map_or(0, |(byte, ch)| byte + ch.len_utf8());
    let prefix = &before[prefix_start..];
    let prefix_xml = block.mapping.get(prefix_start).copied().unwrap_or(offset);
    let before_tokens = tokens(&before[..prefix_start]);
    let qualifier = if before_tokens.last() == Some(&Token::Period) {
        before_tokens
            .get(before_tokens.len().saturating_sub(2))
            .and_then(word)
    } else {
        None
    };
    let table_context = before_tokens
        .last()
        .and_then(word)
        .is_some_and(|s| matches!(s.as_str(), "from" | "join" | "into" | "update"));
    let aliases = relations(&tokens(&block.sql), catalog);
    let mut suggestions = BTreeMap::<String, (String, String, u8)>::new();
    if let Some(qualifier) = qualifier {
        if let Some(key) = aliases.get(&qualifier) {
            if let Some(columns) = catalog.tables.get(key) {
                for (column, kind) in columns {
                    suggestions.insert(
                        column.clone(),
                        (quoted(column), format!("{}.{} · {}", key.0, key.1, kind), 5),
                    );
                }
            }
        } else {
            for (schema, table) in catalog.tables.keys() {
                if *schema == qualifier {
                    suggestions.insert(
                        table.clone(),
                        (quoted(table), format!("{schema}.{table}"), 7),
                    );
                }
            }
        }
    } else {
        if !table_context {
            for key in aliases.values() {
                if let Some(columns) = catalog.tables.get(key) {
                    for (column, kind) in columns {
                        suggestions.entry(column.clone()).or_insert((
                            quoted(column),
                            format!("{}.{} · {}", key.0, key.1, kind),
                            5,
                        ));
                    }
                }
            }
        }
        if table_context || aliases.is_empty() {
            for (schema, table) in catalog.tables.keys() {
                let label = format!("{schema}.{table}");
                suggestions.insert(
                    label,
                    (
                        format!("{}.{}", quoted(schema), quoted(table)),
                        "PostgreSQL 表 / 视图".into(),
                        7,
                    ),
                );
            }
        }
    }
    suggestions.into_iter().filter(|(label,_)|label.to_lowercase().contains(&prefix.to_lowercase())).take(100)
        .map(|(label,(new_text,detail,kind))|json!({"label":label,"kind":kind,"detail":detail,"textEdit":{"range":{"start":sql_text::position(xml,prefix_xml),"end":sql_text::position(xml,offset)},"newText":new_text}})).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alias_and_schema_completions_use_catalog() {
        let mut catalog = Catalog {
            search_path: vec!["public".into()],
            ..Default::default()
        };
        catalog.tables.insert(
            ("public".into(), "stores".into()),
            BTreeMap::from([
                ("id".into(), "bigint".into()),
                ("name".into(), "text".into()),
            ]),
        );
        let xml = "<mapper><select id='x'>SELECT s.na FROM public.stores s</select></mapper>";
        let result = complete(xml, xml.find("s.na").unwrap() + 4, &catalog);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["label"], "name");
        let xml = "<mapper><select id='x'>SELECT * FROM public.st</select></mapper>";
        assert_eq!(
            complete(xml, xml.find("public.st").unwrap() + 9, &catalog)[0]["label"],
            "stores"
        );
    }
}
