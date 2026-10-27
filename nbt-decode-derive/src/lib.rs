use proc_macro::{Delimiter, Group, TokenStream, TokenTree};
use std::str::FromStr;

#[proc_macro_derive(NbtDecode, attributes(nbt))]
pub fn derive_nbt_decode(input: TokenStream) -> TokenStream {
    match expand(input) {
        Ok(output) => TokenStream::from_str(&output).unwrap_or_else(|_| {
            "compile_error!(\"NbtDecode derive generated invalid Rust\");"
                .parse()
                .unwrap()
        }),
        Err(message) => format!("compile_error!({message:?});").parse().unwrap(),
    }
}

struct Field {
    ident: String,
    ty: String,
    wire_name: String,
    skip: bool,
    flatten: bool,
    exported: bool,
}

fn expand(input: TokenStream) -> Result<String, String> {
    let tokens = input.into_iter().collect::<Vec<_>>();
    let struct_index = tokens
        .iter()
        .position(|token| matches!(token, TokenTree::Ident(ident) if ident.to_string() == "struct"))
        .ok_or_else(|| "NbtDecode can only be derived for structs".to_owned())?;
    let name = match tokens.get(struct_index + 1) {
        Some(TokenTree::Ident(ident)) => ident.to_string(),
        _ => return Err("NbtDecode could not read struct name".to_owned()),
    };
    let go_type = parse_container_type_name(&tokens[..struct_index]);
    let body_index = tokens[struct_index + 2..]
        .iter()
        .position(|token| matches!(token, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace))
        .ok_or_else(|| "NbtDecode requires a struct with named fields".to_owned())?;
    let (impl_generics, type_generics, where_clause) =
        parse_generics(&tokens[struct_index + 2..struct_index + 2 + body_index])?;
    let body = match &tokens[struct_index + 2 + body_index] {
        TokenTree::Group(group) => group,
        _ => unreachable!(),
    };
    let fields = parse_fields(body)?;
    let field_bounds = fields
        .iter()
        .filter(|field| !field.skip && field.exported)
        .map(|field| format!("{}: ::rust_water_structure::nbt::NbtDecode", field.ty))
        .collect::<Vec<_>>();
    let mut bounds = field_bounds;
    if !where_clause.is_empty() {
        bounds.push(where_clause);
    }
    let where_clause = if bounds.is_empty() {
        String::new()
    } else {
        format!("where {}", bounds.join(", "))
    };
    let mut decode_dispatch = String::new();
    let mut field_dispatch = String::new();
    for field in fields
        .iter()
        .rev()
        .filter(|field| !field.skip && field.exported)
    {
        if field.flatten {
            decode_dispatch.push_str(&format!(
                "if self.{ident}.decode_compound_field(decoder, tag, raw_name, full_name)? {{ return Ok(()); }};",
                ident = field.ident
            ));
            field_dispatch.push_str(&format!(
                "if self.{ident}.decode_compound_field(decoder, tag, raw_name, full_name)? {{ return Ok(true); }};",
                ident = field.ident
            ));
        } else {
            decode_dispatch.push_str(&format!(
                "if raw_name == {wire:?} {{ return decoder.decode_value(tag, full_name, &mut self.{ident}); }};",
                wire = field.wire_name,
                ident = field.ident
            ));
            field_dispatch.push_str(&format!(
                "if raw_name == {wire:?} {{ return decoder.decode_value(tag, full_name, &mut self.{ident}).map(|()| true); }};",
                wire = field.wire_name,
                ident = field.ident
            ));
        }
    }
    Ok(format!(
        "impl {impl_generics} ::rust_water_structure::nbt::NbtDecode for {name}{type_generics} {where_clause} {{
            fn go_type() -> ::std::string::String {{ {go_type} }}
            fn decode_tag<R: ::std::io::Read>(&mut self, decoder: &mut ::rust_water_structure::nbt::Decoder<R>, tag: u8, name: &str) -> ::rust_water_structure::Result<()> {{
                if tag != 10 {{ return Err(decoder.invalid_struct_type(tag, name, &Self::go_type())); }}
                decoder.decode_compound(name, |decoder, tag, raw_name, full_name| {{
                    {decode_dispatch} Err(decoder.unexpected_named_tag(full_name, tag))
                }})
            }}
            fn decode_compound_field<R: ::std::io::Read>(&mut self, decoder: &mut ::rust_water_structure::nbt::Decoder<R>, tag: u8, raw_name: &str, full_name: &str) -> ::rust_water_structure::Result<bool> {{
                {field_dispatch} Ok(false)
            }}
        }}",
    ))
}

fn parse_container_type_name(tokens: &[TokenTree]) -> String {
    for pair in tokens.windows(2) {
        if !matches!(&pair[0], TokenTree::Punct(punct) if punct.as_char() == '#') {
            continue;
        }
        let TokenTree::Group(attribute) = &pair[1] else {
            continue;
        };
        if attribute.delimiter() != Delimiter::Bracket {
            continue;
        }
        let mut attribute_tokens = attribute.stream().into_iter();
        if !matches!(attribute_tokens.next(), Some(TokenTree::Ident(ref ident)) if ident.to_string() == "nbt")
        {
            continue;
        }
        let Some(TokenTree::Group(settings)) = attribute_tokens.next() else {
            continue;
        };
        if settings.delimiter() != Delimiter::Parenthesis {
            continue;
        }
        let settings = settings.stream().into_iter().collect::<Vec<_>>();
        for setting in settings.windows(3) {
            if matches!(&setting[0], TokenTree::Ident(ident) if ident.to_string() == "type_name")
                && matches!(&setting[1], TokenTree::Punct(punct) if punct.as_char() == '=')
            {
                if let TokenTree::Literal(literal) = &setting[2] {
                    let name = literal.to_string();
                    if let Some(name) = name
                        .strip_prefix('"')
                        .and_then(|name| name.strip_suffix('"'))
                    {
                        return format!("{:?}.to_owned()", name);
                    }
                }
            }
        }
    }
    "::core::any::type_name::<Self>().to_owned()".to_owned()
}

fn parse_fields(body: &Group) -> Result<Vec<Field>, String> {
    let mut fields = Vec::new();
    let mut segment = Vec::new();
    let mut generic_depth = 0usize;
    for token in body.stream() {
        if matches!(&token, TokenTree::Punct(punct) if punct.as_char() == '<') {
            generic_depth += 1;
            segment.push(token);
        } else if matches!(&token, TokenTree::Punct(punct) if punct.as_char() == '>') {
            generic_depth = generic_depth.saturating_sub(1);
            segment.push(token);
        } else if matches!(&token, TokenTree::Punct(punct) if punct.as_char() == ',' && generic_depth == 0)
        {
            if !segment.is_empty() {
                fields.push(parse_field(std::mem::take(&mut segment))?);
            }
        } else {
            segment.push(token);
        }
    }
    if !segment.is_empty() {
        fields.push(parse_field(segment)?);
    }
    Ok(fields)
}

fn parse_field(tokens: Vec<TokenTree>) -> Result<Field, String> {
    let mut skip = false;
    let mut flatten = false;
    let mut rename = None;
    let mut index = 0;
    while index + 1 < tokens.len() {
        if matches!(&tokens[index], TokenTree::Punct(punct) if punct.as_char() == '#') {
            if let TokenTree::Group(attribute) = &tokens[index + 1] {
                if attribute.delimiter() == Delimiter::Bracket {
                    let mut parts = attribute.stream().into_iter();
                    if matches!(parts.next(), Some(TokenTree::Ident(ref ident)) if ident.to_string() == "nbt")
                    {
                        let settings = match parts.next() {
                            Some(TokenTree::Group(group))
                                if group.delimiter() == Delimiter::Parenthesis =>
                            {
                                group.stream().into_iter().collect::<Vec<_>>()
                            }
                            _ => Vec::new(),
                        };
                        for pair in settings.windows(3) {
                            if matches!(&pair[1], TokenTree::Punct(punct) if punct.as_char() == '=')
                            {
                                if matches!(&pair[0], TokenTree::Ident(ident) if ident.to_string() == "rename")
                                {
                                    if let TokenTree::Literal(literal) = &pair[2] {
                                        rename =
                                            Some(literal.to_string().trim_matches('"').to_owned());
                                    }
                                }
                            }
                        }
                        for token in settings {
                            if let TokenTree::Ident(ident) = token {
                                match ident.to_string().as_str() {
                                    "skip" => skip = true,
                                    "flatten" => flatten = true,
                                    _ => {}
                                }
                            }
                        }
                    }
                    index += 2;
                }
            }
        }
        index += 1;
    }
    let colon = tokens
        .iter()
        .position(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ':'))
        .ok_or_else(|| "NbtDecode requires named fields".to_owned())?;
    let ident = tokens[..colon]
        .iter()
        .rev()
        .find_map(|token| match token {
            TokenTree::Ident(ident) if ident.to_string() != "pub" => Some(ident.to_string()),
            _ => None,
        })
        .ok_or_else(|| "NbtDecode could not read field name".to_owned())?;
    let is_public = tokens[..colon]
        .iter()
        .any(|token| matches!(token, TokenTree::Ident(ident) if ident.to_string() == "pub"));
    let ty = tokens[colon + 1..]
        .iter()
        .cloned()
        .collect::<TokenStream>()
        .to_string();
    let wire_name = rename.unwrap_or_else(|| go_field_name(&ident));
    let exported = is_public
        && go_field_name(&ident)
            .chars()
            .next()
            .is_some_and(char::is_uppercase);
    let skip = skip || wire_name == "-";
    let wire_name = wire_name.trim_end_matches(",omitempty");
    let wire_name = if wire_name.is_empty() {
        ident.clone()
    } else {
        wire_name.to_owned()
    };
    Ok(Field {
        wire_name,
        ident,
        ty,
        skip,
        flatten,
        exported,
    })
}

fn go_field_name(rust_name: &str) -> String {
    let mut name = String::new();
    let mut uppercase = true;
    for character in rust_name.strip_prefix("r#").unwrap_or(rust_name).chars() {
        if character == '_' {
            uppercase = true;
        } else if uppercase {
            name.extend(character.to_uppercase());
            uppercase = false;
        } else {
            name.push(character);
        }
    }
    name
}

fn parse_generics(tokens: &[TokenTree]) -> Result<(String, String, String), String> {
    if !matches!(tokens.first(), Some(TokenTree::Punct(punct)) if punct.as_char() == '<') {
        let where_clause = tokens
            .iter()
            .skip_while(
                |token| !matches!(token, TokenTree::Ident(ident) if ident.to_string() == "where"),
            )
            .cloned()
            .collect::<TokenStream>()
            .to_string()
            .trim_start_matches("where ")
            .to_owned();
        return Ok((String::new(), String::new(), where_clause));
    }
    let mut depth = 0usize;
    let mut closing = None;
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token, TokenTree::Punct(punct) if punct.as_char() == '<') {
            depth += 1;
        } else if matches!(token, TokenTree::Punct(punct) if punct.as_char() == '>') {
            depth -= 1;
            if depth == 0 {
                closing = Some(index);
                break;
            }
        }
    }
    let closing =
        closing.ok_or_else(|| "NbtDecode could not parse generic parameters".to_owned())?;
    let params = &tokens[1..closing];
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut nested = 0usize;
    for token in params {
        if matches!(token, TokenTree::Punct(punct) if punct.as_char() == '<') {
            nested += 1;
            current.push(token.clone());
        } else if matches!(token, TokenTree::Punct(punct) if punct.as_char() == '>') {
            nested = nested.saturating_sub(1);
            current.push(token.clone());
        } else if matches!(token, TokenTree::Punct(punct) if punct.as_char() == ',' && nested == 0)
        {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
        } else {
            current.push(token.clone());
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    let mut impl_params = Vec::new();
    let mut type_params = Vec::new();
    for mut segment in segments {
        let mut nested = 0usize;
        if let Some(default_index) = segment.iter().position(|token| {
            if matches!(token, TokenTree::Punct(punct) if punct.as_char() == '<') {
                nested += 1;
            } else if matches!(token, TokenTree::Punct(punct) if punct.as_char() == '>') {
                nested = nested.saturating_sub(1);
            }
            matches!(token, TokenTree::Punct(punct) if punct.as_char() == '=' && nested == 0)
        }) {
            segment.truncate(default_index);
        }
        let kind = match segment.first() {
            Some(TokenTree::Punct(punct)) if punct.as_char() == '\'' => {
                let lifetime = segment
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<TokenStream>()
                    .to_string();
                Some(lifetime)
            }
            Some(TokenTree::Ident(ident)) if ident.to_string() == "const" => {
                segment.iter().nth(1).map(|ident| ident.to_string())
            }
            Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
            _ => None,
        }
        .ok_or_else(|| "NbtDecode could not parse a generic parameter".to_owned())?;
        impl_params.push(segment.into_iter().collect::<TokenStream>().to_string());
        type_params.push(kind);
    }
    let where_clause = tokens[closing + 1..]
        .iter()
        .skip_while(
            |token| !matches!(token, TokenTree::Ident(ident) if ident.to_string() == "where"),
        )
        .skip(1)
        .cloned()
        .collect::<TokenStream>()
        .to_string();
    Ok((
        format!("<{}>", impl_params.join(", ")),
        format!("<{}>", type_params.join(", ")),
        where_clause,
    ))
}
