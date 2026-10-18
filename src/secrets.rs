use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::config::Secret;
use crate::error::Error;
use crate::i18n::tr;

const GROUP_AND_OTHERS: u32 = 0o077;

/// Что не так с файлом парольной фразы; `None`, если файлом можно пользоваться.
pub fn problem(path: &str) -> Option<String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return Some(format!("{path}: {error}")),
    };

    if !metadata.is_file() {
        return Some(tr!(
            "{path}: not a regular file",
            "{path}: это не обычный файл"
        ));
    }

    if metadata.permissions().mode() & GROUP_AND_OTHERS != 0 {
        return Some(tr!(
            "{path}: the file is accessible to other users, chmod 600 is required",
            "{path}: файл доступен другим пользователям, нужен chmod 600"
        ));
    }

    None
}

/// Парольная фраза из файла: одна строка, один завершающий перевод строки отбрасывается.
pub fn read_passphrase(path: &str) -> Result<Secret, Error> {
    if let Some(problem) = problem(path) {
        return Err(Error::Invalid(problem));
    }

    let text =
        fs::read_to_string(path).map_err(|error| Error::Invalid(format!("{path}: {error}")))?;
    let phrase = text.strip_suffix('\n').unwrap_or(&text);

    if phrase.is_empty() || phrase.contains('\n') {
        return Err(Error::Invalid(tr!(
            "{path}: the file must contain exactly one non-empty line",
            "{path}: в файле должна быть одна непустая строка"
        )));
    }

    Ok(Secret::new(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, contents: &str, mode: u32) -> String {
        let path =
            std::env::temp_dir().join(format!("moonshine-secret-{name}-{}", std::process::id()));

        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn the_phrase_loses_only_one_trailing_newline() {
        let path = file("plain", "correct horse\n", 0o600);

        assert_eq!(read_passphrase(&path).unwrap().expose(), "correct horse");
        assert!(problem(&path).is_none());
    }

    #[test]
    fn open_permissions_empty_and_multiline_files_are_refused() {
        for (name, contents, mode) in [
            ("open", "phrase\n", 0o644),
            ("empty", "\n", 0o600),
            ("lines", "one\ntwo\n", 0o600),
        ] {
            assert!(
                read_passphrase(&file(name, contents, mode)).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_missing_file_and_a_directory_are_problems() {
        assert!(problem("/nonexistent/phrase").is_some());
        assert!(problem("/tmp").is_some());
    }
}
