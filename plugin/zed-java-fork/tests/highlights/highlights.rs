#[cfg(test)]
mod tests {
    use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

    fn captures(source: &str) -> Vec<(String, String)> {
        let language = tree_sitter_java::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).expect("Java grammar");
        let tree = parser.parse(source, None).expect("Java syntax tree");
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        let query = Query::new(
            &language,
            include_str!("../../languages/java/highlights.scm"),
        )
        .expect("Java highlight query");
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut result = Vec::new();
        while let Some(matched) = matches.next() {
            for capture in matched.captures {
                result.push((
                    capture
                        .node
                        .utf8_text(source.as_bytes())
                        .expect("capture text")
                        .into(),
                    query.capture_names()[capture.index as usize].into(),
                ));
            }
        }
        result
    }

    #[test]
    fn annotation_symbols_and_qualified_names_use_metadata_color() {
        let captures = captures(
            "@Transactional(readOnly = true, isolation = Isolation.REPEATABLE_READ)\n\
             @Override @org.example.Marker @org.example.Annotation(value = 1)\n\
             class Example {}",
        );
        assert_eq!(
            captures
                .iter()
                .filter(|(text, style)| text == "@" && style == "attribute")
                .count(),
            4
        );
        for name in [
            "Transactional",
            "Override",
            "org.example.Marker",
            "org.example.Annotation",
        ] {
            assert!(
                captures.contains(&(name.into(), "attribute".into())),
                "{name}"
            );
        }
        assert!(
            !captures
                .iter()
                .any(|(text, style)| text == "@" && style == "punctuation.special")
        );
        for (text, style) in [
            ("true", "boolean"),
            ("=", "operator"),
            ("(", "punctuation.bracket"),
        ] {
            assert!(captures.contains(&(text.into(), style.into())));
        }
    }

    #[test]
    fn java_keyword_forms_do_not_use_type_variable_or_constant_colors() {
        let captures = captures(
            "class Example extends Base { boolean b; int i; double d;\n\
             void run() { var value = this; Object other = null; super.run(); } }",
        );
        for text in [
            "boolean", "int", "double", "void", "var", "this", "null", "super",
        ] {
            let effective = captures.iter().rev().find(|(captured, _)| captured == text);
            assert_eq!(
                effective.map(|(_, style)| style.as_str()),
                Some("keyword"),
                "{text}"
            );
        }
    }
}
