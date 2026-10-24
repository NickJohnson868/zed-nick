use quick_xml::{Reader, events::Event};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct SqlBlock {
    pub sql: String,
    pub mapping: Vec<usize>,
    pub start: usize,
    pub end: usize,
    pub dynamic: bool,
}

impl SqlBlock {
    pub fn sql_offset(&self, xml_offset: usize) -> usize {
        self.mapping.partition_point(|source| *source < xml_offset)
    }

    fn append(&mut self, raw: &str, start: usize, cdata: bool) {
        if self.sql.len() + raw.len() > 256 * 1024 {
            self.dynamic = true;
            return;
        }
        let mut index = 0;
        while index < raw.len() {
            if !cdata
                && raw.as_bytes()[index] == b'&'
                && let Some(end) = raw[index..].find(';')
                && let Ok(decoded) = quick_xml::escape::unescape(&raw[index..index + end + 1])
            {
                self.sql.push_str(&decoded);
                self.mapping
                    .extend(std::iter::repeat_n(start + index, decoded.len()));
                index += end + 1;
                continue;
            }
            let ch = raw[index..].chars().next().unwrap();
            self.sql.push(ch);
            self.mapping
                .extend((0..ch.len_utf8()).map(|byte| start + index + byte));
            index += ch.len_utf8();
        }
    }

    fn parameters(&mut self) {
        if self.sql.contains("${") {
            self.dynamic = true;
            return;
        }
        let old = std::mem::take(&mut self.sql);
        let mapping = std::mem::take(&mut self.mapping);
        let mut index = 0;
        let mut parameter = 0;
        while index < old.len() {
            if old[index..].starts_with("#{") {
                let Some(end) = old[index + 2..].find('}') else {
                    self.dynamic = true;
                    return;
                };
                parameter += 1;
                let replacement = format!("${parameter}");
                self.sql.push_str(&replacement);
                self.mapping
                    .extend(std::iter::repeat_n(mapping[index], replacement.len()));
                index += end + 3;
            } else {
                let ch = old[index..].chars().next().unwrap();
                self.sql.push(ch);
                self.mapping
                    .extend_from_slice(&mapping[index..index + ch.len_utf8()]);
                index += ch.len_utf8();
            }
        }
    }
}

pub fn blocks(xml: &str) -> Vec<SqlBlock> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut mapper = false;
    let mut current: Option<SqlBlock> = None;
    let mut result = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                if depth == 0 {
                    mapper = event.name().as_ref() == b"mapper";
                }
                if mapper
                    && depth == 1
                    && matches!(
                        event.name().as_ref(),
                        b"select" | b"insert" | b"update" | b"delete"
                    )
                {
                    current = Some(SqlBlock {
                        sql: String::new(),
                        mapping: Vec::new(),
                        start: reader.buffer_position() as usize,
                        end: 0,
                        dynamic: false,
                    });
                } else if let Some(block) = &mut current {
                    block.dynamic = true;
                }
                depth += 1;
            }
            Ok(Event::Empty(_)) => {
                if let Some(block) = &mut current {
                    block.dynamic = true;
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
                if depth == 1
                    && let Some(mut block) = current.take()
                {
                    block.end = start;
                    block.parameters();
                    if block.sql.len() <= 256 * 1024 {
                        result.push(block);
                    }
                }
            }
            Ok(Event::Text(text)) => {
                if let Some(block) = &mut current {
                    block.append(std::str::from_utf8(&text).unwrap_or(""), start, false);
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(block) = &mut current {
                    block.append(std::str::from_utf8(&text).unwrap_or(""), start + 9, true);
                }
            }
            Ok(Event::Comment(_)) => {
                if let Some(block) = &mut current {
                    block.sql.push(' ');
                    block.mapping.push(start);
                }
            }
            Ok(Event::Eof) => return result,
            Err(_) => return Vec::new(),
            _ => {}
        }
    }
}

pub fn position(xml: &str, offset: usize) -> Value {
    let offset = offset.min(xml.len());
    let text = &xml[..offset];
    json!({"line":text.bytes().filter(|b|*b==b'\n').count(),"character":text.rsplit('\n').next().unwrap_or("").encode_utf16().count()})
}

pub fn diagnostic(
    xml: &str,
    block: &SqlBlock,
    character: usize,
    message: &str,
    code: &str,
) -> Value {
    let sql_byte = block
        .sql
        .char_indices()
        .nth(character.saturating_sub(1))
        .map_or(block.sql.len(), |(byte, _)| byte);
    let start = block.mapping.get(sql_byte).copied().unwrap_or(block.end);
    let tail = &xml[start..];
    let length = tail
        .char_indices()
        .take_while(|(_, ch)| ch.is_alphanumeric() || matches!(ch, '_' | '.' | '"'))
        .last()
        .map_or(1, |(i, ch)| i + ch.len_utf8());
    let end = (start + length).min(xml.len());
    json!({"range":{"start":position(xml,start),"end":position(xml,end)},"severity":1,"code":code,"source":"MyBatis PostgreSQL","message":message})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholders_entities_and_cdata_keep_xml_positions() {
        let xml = "<mapper>\n<select id='x'>SELECT missing FROM t WHERE id = #{id} AND x &lt; 5 <![CDATA[AND y > 1]]></select></mapper>";
        let result = blocks(xml);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].sql,
            "SELECT missing FROM t WHERE id = $1 AND x < 5 AND y > 1"
        );
        let d = diagnostic(xml, &result[0], 8, "missing", "42703");
        assert_eq!(d["range"]["start"]["character"], 22);
    }
    #[test]
    fn dynamic_sql_is_skipped_as_a_whole() {
        for body in [
            "SELECT * FROM t <if test='x'>WHERE bad=1</if>",
            "SELECT <include refid='x'/> FROM t",
            "SELECT ${column} FROM t",
        ] {
            let xml = format!("<mapper><select id='x'>{body}</select></mapper>");
            assert!(blocks(&xml)[0].dynamic);
        }
    }
}
