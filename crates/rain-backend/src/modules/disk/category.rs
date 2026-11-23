//! The colour class of a file, decided by its extension. Section 05 6.6.
//!
//! One `match`, compiled in: nobody asked to change the table, so it is not a data file.

use rain_types::FileCategory;

/// The categories in a fixed order. The walker keeps its byte counts in an array with this
/// order, so a category and its index must always agree (`index_of`).
pub const CATEGORIES: [FileCategory; 8] = [
    FileCategory::Video,
    FileCategory::Image,
    FileCategory::Audio,
    FileCategory::Archive,
    FileCategory::Code,
    FileCategory::Document,
    FileCategory::App,
    FileCategory::Other,
];

/// The position of a category in `CATEGORIES`.
pub fn index_of(category: FileCategory) -> usize {
    match category {
        FileCategory::Video => 0,
        FileCategory::Image => 1,
        FileCategory::Audio => 2,
        FileCategory::Archive => 3,
        FileCategory::Code => 4,
        FileCategory::Document => 5,
        FileCategory::App => 6,
        FileCategory::Other => 7,
    }
}

/// The longest extension that is looked up; a longer one is certainly not on the list.
const EXTENSION_MAX: usize = 8;

/// The category of a file name. The extension is the text after the last dot, but only when
/// something comes before that dot (`.gitignore` and `Makefile` have none).
pub fn category_of(file_name: &str) -> FileCategory {
    let Some((stem, extension)) = file_name.rsplit_once('.') else {
        return FileCategory::Other;
    };
    if stem.is_empty() || extension.len() > EXTENSION_MAX {
        return FileCategory::Other;
    }
    match extension.to_ascii_lowercase().as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "m4v" | "flv" | "mpg" | "mpeg"
        | "m2ts" | "mts" | "bik" => FileCategory::Video,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "heic" | "heif" | "tif" | "tiff"
        | "svg" | "ico" | "raw" | "cr2" | "nef" | "arw" | "dng" | "psd" | "xcf" => {
            FileCategory::Image
        }
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "opus" | "m4a" | "wma" | "aiff" | "mid" => {
            FileCategory::Audio
        }
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "iso" | "img"
        | "vhd" | "vhdx" | "vmdk" | "cab" | "wim" | "pak" => FileCategory::Archive,
        // `ts` is code, not video: MPEG transport streams use `m2ts` and `mts`.
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "py" | "go" | "c" | "h"
        | "cpp" | "hpp" | "cs" | "java" | "kt" | "swift" | "rb" | "php" | "sh" | "ps1" | "bat"
        | "cmd" | "json" | "toml" | "yaml" | "yml" | "xml" | "html" | "css" | "scss" | "sql"
        | "lock" => FileCategory::Code,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "txt" | "md"
        | "rtf" | "csv" | "epub" => FileCategory::Document,
        "exe" | "dll" | "msi" | "sys" | "msix" | "appx" | "lib" | "pdb" | "obj" => {
            FileCategory::App
        }
        _ => FileCategory::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FileCategory::*;

    #[test]
    fn every_extension_of_the_table_has_its_category() {
        let table: [(FileCategory, &str); 7] = [
            (
                Video,
                "mp4 mkv avi mov wmv webm m4v flv mpg mpeg m2ts mts bik",
            ),
            (
                Image,
                "jpg jpeg png gif bmp webp heic heif tif tiff svg ico raw cr2 nef arw dng psd xcf",
            ),
            (Audio, "mp3 wav flac aac ogg opus m4a wma aiff mid"),
            (
                Archive,
                "zip 7z rar tar gz tgz bz2 xz zst iso img vhd vhdx vmdk cab wim pak",
            ),
            (
                Code,
                "rs ts tsx js jsx mjs cjs vue py go c h cpp hpp cs java kt swift rb php sh ps1 bat cmd json toml yaml yml xml html css scss sql lock",
            ),
            (
                Document,
                "pdf doc docx xls xlsx ppt pptx odt ods txt md rtf csv epub",
            ),
            (App, "exe dll msi sys msix appx lib pdb obj"),
        ];
        for (category, extensions) in table {
            for extension in extensions.split(' ') {
                assert_eq!(
                    category_of(&format!("file.{extension}")),
                    category,
                    "{extension}"
                );
            }
        }
    }

    #[test]
    fn the_extension_is_compared_in_lower_case() {
        assert_eq!(category_of("A.MP4"), Video);
        assert_eq!(category_of("Photo.JpG"), Image);
    }

    #[test]
    fn names_without_an_extension_are_other() {
        assert_eq!(category_of(".gitignore"), Other);
        assert_eq!(category_of("Makefile"), Other);
        assert_eq!(category_of("ends-with-a-dot."), Other);
        assert_eq!(category_of("data.unknownext"), Other);
    }

    #[test]
    fn only_the_last_extension_counts_and_long_ones_are_other() {
        assert_eq!(category_of("archive.tar.gz"), Archive);
        assert_eq!(category_of("notes.txt.exe"), App);
        assert_eq!(category_of("file.abcdefghi"), Other, "nine letters");
        assert_eq!(category_of("file.fileinfo"), Other);
    }

    #[test]
    fn ts_is_code_and_lock_is_code() {
        assert_eq!(category_of("main.ts"), Code);
        assert_eq!(category_of("Cargo.lock"), Code);
    }

    #[test]
    fn the_index_matches_the_order_of_the_array() {
        for (position, category) in CATEGORIES.iter().enumerate() {
            assert_eq!(index_of(*category), position);
        }
    }
}
