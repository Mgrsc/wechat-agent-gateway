pub fn redact_id(value: &str) -> String {
    if value.is_empty() {
        return "<empty>".to_string();
    }

    if value.len() <= 8 {
        return "<redacted>".to_string();
    }

    format!("{}***{}", &value[..4], &value[value.len() - 4..])
}

pub fn redact_optional_id(value: Option<&str>) -> Option<String> {
    value.map(redact_id)
}

pub fn redact_text(value: &str) -> String {
    format!("<len:{}>", value.chars().count())
}

pub fn sanitize_url(value: &str) -> String {
    value.split('?').next().unwrap_or(value).to_string()
}
