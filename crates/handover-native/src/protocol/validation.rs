pub(crate) fn safe_share_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

pub(crate) fn safe_browse_path(path: &str) -> bool {
    if path == "." {
        return true;
    }
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\\'))
}

pub(crate) fn valid_transfer_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn valid_share_url(url: &str) -> bool {
    if url.is_empty()
        || url.len() > 8192
        || url.trim() != url
        || url.chars().any(char::is_whitespace)
        || url.chars().any(char::is_control)
    {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    !matches!(parsed.scheme(), "file" | "javascript" | "data")
}
