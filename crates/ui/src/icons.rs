//! Icons bundled with diffz itself; they are served before anything gpui-kit bundles, so
//! toolbar glyphs keep one visual style, never dropping into mismatching text fonts.
use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    WrapText,
    Columns2,
    Rows2,
    MessageSquare,
    File,
    FileRust,
    FileJavascript,
    FileReact,
    FileTypescript,
    FileReactTs,
    FileJson,
    FilePython,
    FileGo,
    FileC,
    FileCpp,
    FileJava,
    FileRuby,
    FileShell,
    FileToml,
    FileYaml,
    FileHtml,
    FileCss,
    FileElixir,
    FileGleam,
    FileMarkdown,
    FileHandlebars,
    FileImage,
    FileDiff,
    FileDatabase,
    FilePdf,
    FileXml,
    FileArchive,
    FileAudio,
    FileVideo,
    FileFont,
    FileLicense,
    FileMakefile,
    FileDocker,
    FileGit,
    FileGithubActions,
    FileNpm,
    FileSettings,
    FileLock,
}
impl AppIcon {
    pub const ALL: [AppIcon; 43] = [
        AppIcon::WrapText,
        AppIcon::Columns2,
        AppIcon::Rows2,
        AppIcon::MessageSquare,
        AppIcon::File,
        AppIcon::FileRust,
        AppIcon::FileJavascript,
        AppIcon::FileReact,
        AppIcon::FileTypescript,
        AppIcon::FileReactTs,
        AppIcon::FileJson,
        AppIcon::FilePython,
        AppIcon::FileGo,
        AppIcon::FileC,
        AppIcon::FileCpp,
        AppIcon::FileJava,
        AppIcon::FileRuby,
        AppIcon::FileShell,
        AppIcon::FileToml,
        AppIcon::FileYaml,
        AppIcon::FileHtml,
        AppIcon::FileCss,
        AppIcon::FileElixir,
        AppIcon::FileGleam,
        AppIcon::FileMarkdown,
        AppIcon::FileHandlebars,
        AppIcon::FileImage,
        AppIcon::FileDiff,
        AppIcon::FileDatabase,
        AppIcon::FilePdf,
        AppIcon::FileXml,
        AppIcon::FileArchive,
        AppIcon::FileAudio,
        AppIcon::FileVideo,
        AppIcon::FileFont,
        AppIcon::FileLicense,
        AppIcon::FileMakefile,
        AppIcon::FileDocker,
        AppIcon::FileGit,
        AppIcon::FileGithubActions,
        AppIcon::FileNpm,
        AppIcon::FileSettings,
        AppIcon::FileLock,
    ];
    pub fn for_path(path: &str) -> Self {
        let lower_path = path.to_ascii_lowercase().replace('\\', "/");
        let basename = lower_path.rsplit('/').next().unwrap_or(&lower_path);
        let extension = basename.rsplit_once('.').map(|(_, extension)| extension);
        let is_name_variant = |name: &str| {
            basename == name
                || basename
                    .strip_prefix(name)
                    .is_some_and(|suffix| suffix.starts_with('.') || suffix.starts_with('-'))
        };

        if is_name_variant("readme") {
            return Self::FileMarkdown;
        }
        if is_name_variant("license")
            || is_name_variant("licence")
            || is_name_variant("copying")
            || is_name_variant("copyright")
        {
            return Self::FileLicense;
        }
        if is_name_variant("makefile") || is_name_variant("gnumakefile") {
            return Self::FileMakefile;
        }
        if is_name_variant("dockerfile") || is_name_variant("containerfile") {
            return Self::FileDocker;
        }
        if (lower_path.starts_with(".github/workflows/")
            || lower_path.contains("/.github/workflows/"))
            && matches!(extension, Some("yml" | "yaml"))
        {
            return Self::FileGithubActions;
        }
        if matches!(basename, "cargo.toml" | "cargo.lock") {
            return Self::FileRust;
        }
        if matches!(
            basename,
            "package.json" | "package-lock.json" | "npm-shrinkwrap.json" | ".npmrc" | ".npmignore"
        ) {
            return Self::FileNpm;
        }
        if basename.starts_with("tsconfig") && extension == Some("json") {
            return Self::FileTypescript;
        }
        if matches!(basename, "go.mod" | "go.sum" | "go.work" | "go.work.sum") {
            return Self::FileGo;
        }
        if is_name_variant("gemfile") || is_name_variant("rakefile") {
            return Self::FileRuby;
        }
        if basename == "mix.exs" {
            return Self::FileElixir;
        }
        if basename == "gleam.toml" {
            return Self::FileGleam;
        }
        if basename == "pkgbuild" {
            return Self::FileShell;
        }
        if basename.starts_with(".env") || basename == ".editorconfig" {
            return Self::FileSettings;
        }

        match extension {
            Some("rs") => Self::FileRust,
            Some("js" | "mjs" | "cjs" | "es6" | "esx" | "pac") => Self::FileJavascript,
            Some("jsx") => Self::FileReact,
            Some("ts" | "mts" | "cts") => Self::FileTypescript,
            Some("tsx") => Self::FileReactTs,
            Some(
                "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "geojson" | "har" | "jsonld"
                | "webmanifest" | "tsbuildinfo",
            ) => Self::FileJson,
            Some("py" | "cpy" | "gyp" | "gypi" | "ipy" | "pyi" | "pyt" | "pyw" | "rpy") => {
                Self::FilePython
            }
            Some("go") => Self::FileGo,
            Some("c" | "h" | "i" | "mi") => Self::FileC,
            Some(
                "cc" | "cpp" | "cxx" | "c++" | "cp" | "mii" | "ii" | "cppm" | "c++m" | "ccm"
                | "cxxm" | "hpp" | "hh" | "hxx" | "h++" | "hp" | "ipp" | "ixx" | "tpp" | "txx",
            ) => Self::FileCpp,
            Some("java" | "jsp" | "jav") => Self::FileJava,
            Some(
                "rb" | "erb" | "rbs" | "gemspec" | "podspec" | "rake" | "rbi" | "rbx" | "rjs"
                | "ru",
            ) => Self::FileRuby,
            Some(
                "sh" | "ksh" | "csh" | "tcsh" | "zsh" | "bash" | "bat" | "cmd" | "awk" | "fish"
                | "exp" | "nu" | "xsh" | "ps1" | "psm1" | "psd1",
            ) => Self::FileShell,
            Some("toml") => Self::FileToml,
            Some("yaml" | "yml" | "eyaml" | "eyml") => Self::FileYaml,
            Some("html" | "htm" | "xhtml" | "xht" | "shtml" | "aspx" | "asp" | "rhtml") => {
                Self::FileHtml
            }
            Some("css" | "scss" | "sass" | "less") => Self::FileCss,
            Some("ex" | "exs" | "eex" | "leex" | "heex") => Self::FileElixir,
            Some("gleam") => Self::FileGleam,
            Some(
                "md" | "markdown" | "mdx" | "rst" | "copilotmd" | "litcoffee" | "markdn" | "mdown"
                | "mdtext" | "mdtxt" | "mdwn" | "mkd" | "mkdn" | "ronn" | "workbook" | "txt",
            ) => Self::FileMarkdown,
            Some("hbs" | "mustache" | "handlebars" | "hjs") => Self::FileHandlebars,
            Some(
                "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "tif" | "tiff" | "avif"
                | "bmp" | "heif" | "heic" | "jxl" | "raw" | "tga" | "xcf" | "icns",
            ) => Self::FileImage,
            Some("diff" | "patch" | "rej") => Self::FileDiff,
            Some(
                "sql" | "pks" | "pkb" | "accdb" | "mdb" | "sqlite" | "sqlite3" | "pgsql"
                | "postgres" | "plpgsql" | "psql" | "db" | "db3" | "dblite" | "dblite3" | "odb"
                | "dbf" | "fdb" | "gdb" | "ibd" | "mdf" | "myd" | "myi" | "ndf" | "orc" | "parquet"
                | "sdf" | "ldf" | "frm" | "dsql",
            ) => Self::FileDatabase,
            Some("pdf") => Self::FilePdf,
            Some(
                "xml" | "plist" | "xsd" | "dtd" | "xsl" | "xslt" | "resx" | "iml" | "xquery"
                | "manifest" | "project" | "xaml" | "axaml" | "axml" | "fxml" | "rss" | "atom"
                | "wsdl" | "xlf",
            ) => Self::FileXml,
            Some(
                "zip" | "z" | "tar" | "gz" | "xz" | "lz" | "lzma" | "lz4" | "lz5" | "lzh" | "lha"
                | "br" | "bz2" | "bzip2" | "gzip" | "brotli" | "7z" | "rar" | "tz" | "taz" | "tlz"
                | "txz" | "tgz" | "tpz" | "tbz" | "tbz2" | "zst" | "zstd" | "tzst" | "tzstd"
                | "cab" | "cpio" | "rpm" | "deb" | "arj" | "wim" | "swm" | "esd" | "xar"
                | "squashfs" | "apfs",
            ) => Self::FileArchive,
            Some(
                "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" | "opus" | "wma" | "aiff" | "ape"
                | "mid" | "midi",
            ) => Self::FileAudio,
            Some(
                "mp4" | "webm" | "mov" | "avi" | "mkv" | "flv" | "wmv" | "m4v" | "mpeg" | "mpg"
                | "ogv" | "3gp",
            ) => Self::FileVideo,
            Some("woff" | "woff2" | "ttf" | "eot" | "otf" | "fnt" | "ttc" | "font" | "fonts") => {
                Self::FileFont
            }
            Some("mk") => Self::FileMakefile,
            Some("dockerignore" | "containerignore" | "dockerfile" | "containerfile") => {
                Self::FileDocker
            }
            Some(
                "ini" | "dlc" | "config" | "conf" | "properties" | "prop" | "settings" | "option"
                | "props" | "prefs" | "cfg" | "cnf" | "tool-versions" | "directory" | "mak"
                | "repo" | "editorconfig",
            ) => Self::FileSettings,
            Some("gitignore" | "gitattributes" | "gitmodules") => Self::FileGit,
            Some("lock") => Self::FileLock,
            _ => Self::File,
        }
    }
    pub fn path(self) -> &'static str {
        match self {
            AppIcon::WrapText => "icons/diffz/wrap-text.svg",
            AppIcon::Columns2 => "icons/diffz/columns-2.svg",
            AppIcon::Rows2 => "icons/diffz/rows-2.svg",
            AppIcon::MessageSquare => "icons/diffz/message-square.svg",
            AppIcon::File => "icons/diffz/file.svg",
            AppIcon::FileRust => "icons/diffz/file-rust.svg",
            AppIcon::FileJavascript => "icons/diffz/file-javascript.svg",
            AppIcon::FileReact => "icons/diffz/file-react.svg",
            AppIcon::FileTypescript => "icons/diffz/file-typescript.svg",
            AppIcon::FileReactTs => "icons/diffz/file-react-ts.svg",
            AppIcon::FileJson => "icons/diffz/file-json.svg",
            AppIcon::FilePython => "icons/diffz/file-python.svg",
            AppIcon::FileGo => "icons/diffz/file-go.svg",
            AppIcon::FileC => "icons/diffz/file-c.svg",
            AppIcon::FileCpp => "icons/diffz/file-cpp.svg",
            AppIcon::FileJava => "icons/diffz/file-java.svg",
            AppIcon::FileRuby => "icons/diffz/file-ruby.svg",
            AppIcon::FileShell => "icons/diffz/file-shell.svg",
            AppIcon::FileToml => "icons/diffz/file-toml.svg",
            AppIcon::FileYaml => "icons/diffz/file-yaml.svg",
            AppIcon::FileHtml => "icons/diffz/file-html.svg",
            AppIcon::FileCss => "icons/diffz/file-css.svg",
            AppIcon::FileElixir => "icons/diffz/file-elixir.svg",
            AppIcon::FileGleam => "icons/diffz/file-gleam.svg",
            AppIcon::FileMarkdown => "icons/diffz/file-markdown.svg",
            AppIcon::FileHandlebars => "icons/diffz/file-handlebars.svg",
            AppIcon::FileImage => "icons/diffz/file-image.svg",
            AppIcon::FileDiff => "icons/diffz/file-diff.svg",
            AppIcon::FileDatabase => "icons/diffz/file-database.svg",
            AppIcon::FilePdf => "icons/diffz/file-pdf.svg",
            AppIcon::FileXml => "icons/diffz/file-xml.svg",
            AppIcon::FileArchive => "icons/diffz/file-archive.svg",
            AppIcon::FileAudio => "icons/diffz/file-audio.svg",
            AppIcon::FileVideo => "icons/diffz/file-video.svg",
            AppIcon::FileFont => "icons/diffz/file-font.svg",
            AppIcon::FileLicense => "icons/diffz/file-license.svg",
            AppIcon::FileMakefile => "icons/diffz/file-makefile.svg",
            AppIcon::FileDocker => "icons/diffz/file-docker.svg",
            AppIcon::FileGit => "icons/diffz/file-git.svg",
            AppIcon::FileGithubActions => "icons/diffz/file-github-actions.svg",
            AppIcon::FileNpm => "icons/diffz/file-npm.svg",
            AppIcon::FileSettings => "icons/diffz/file-settings.svg",
            AppIcon::FileLock => "icons/diffz/file-lock.svg",
        }
    }
    fn bytes(path: &str) -> Option<&'static [u8]> {
        Some(match path {
            "icons/diffz/wrap-text.svg" => include_bytes!("../icons/wrap-text.svg"),
            "icons/diffz/columns-2.svg" => include_bytes!("../icons/columns-2.svg"),
            "icons/diffz/rows-2.svg" => include_bytes!("../icons/rows-2.svg"),
            "icons/diffz/message-square.svg" => include_bytes!("../icons/message-square.svg"),
            "icons/diffz/file.svg" => include_bytes!("../icons/file.svg"),
            "icons/diffz/file-rust.svg" => include_bytes!("../icons/file-rust.svg"),
            "icons/diffz/file-javascript.svg" => include_bytes!("../icons/file-javascript.svg"),
            "icons/diffz/file-react.svg" => include_bytes!("../icons/file-react.svg"),
            "icons/diffz/file-typescript.svg" => include_bytes!("../icons/file-typescript.svg"),
            "icons/diffz/file-react-ts.svg" => include_bytes!("../icons/file-react-ts.svg"),
            "icons/diffz/file-json.svg" => include_bytes!("../icons/file-json.svg"),
            "icons/diffz/file-python.svg" => include_bytes!("../icons/file-python.svg"),
            "icons/diffz/file-go.svg" => include_bytes!("../icons/file-go.svg"),
            "icons/diffz/file-c.svg" => include_bytes!("../icons/file-c.svg"),
            "icons/diffz/file-cpp.svg" => include_bytes!("../icons/file-cpp.svg"),
            "icons/diffz/file-java.svg" => include_bytes!("../icons/file-java.svg"),
            "icons/diffz/file-ruby.svg" => include_bytes!("../icons/file-ruby.svg"),
            "icons/diffz/file-shell.svg" => include_bytes!("../icons/file-shell.svg"),
            "icons/diffz/file-toml.svg" => include_bytes!("../icons/file-toml.svg"),
            "icons/diffz/file-yaml.svg" => include_bytes!("../icons/file-yaml.svg"),
            "icons/diffz/file-html.svg" => include_bytes!("../icons/file-html.svg"),
            "icons/diffz/file-css.svg" => include_bytes!("../icons/file-css.svg"),
            "icons/diffz/file-elixir.svg" => include_bytes!("../icons/file-elixir.svg"),
            "icons/diffz/file-gleam.svg" => include_bytes!("../icons/file-gleam.svg"),
            "icons/diffz/file-markdown.svg" => include_bytes!("../icons/file-markdown.svg"),
            "icons/diffz/file-handlebars.svg" => include_bytes!("../icons/file-handlebars.svg"),
            "icons/diffz/file-image.svg" => include_bytes!("../icons/file-image.svg"),
            "icons/diffz/file-diff.svg" => include_bytes!("../icons/file-diff.svg"),
            "icons/diffz/file-database.svg" => include_bytes!("../icons/file-database.svg"),
            "icons/diffz/file-pdf.svg" => include_bytes!("../icons/file-pdf.svg"),
            "icons/diffz/file-xml.svg" => include_bytes!("../icons/file-xml.svg"),
            "icons/diffz/file-archive.svg" => include_bytes!("../icons/file-archive.svg"),
            "icons/diffz/file-audio.svg" => include_bytes!("../icons/file-audio.svg"),
            "icons/diffz/file-video.svg" => include_bytes!("../icons/file-video.svg"),
            "icons/diffz/file-font.svg" => include_bytes!("../icons/file-font.svg"),
            "icons/diffz/file-license.svg" => include_bytes!("../icons/file-license.svg"),
            "icons/diffz/file-makefile.svg" => include_bytes!("../icons/file-makefile.svg"),
            "icons/diffz/file-docker.svg" => include_bytes!("../icons/file-docker.svg"),
            "icons/diffz/file-git.svg" => include_bytes!("../icons/file-git.svg"),
            "icons/diffz/file-github-actions.svg" => {
                include_bytes!("../icons/file-github-actions.svg")
            }
            "icons/diffz/file-npm.svg" => include_bytes!("../icons/file-npm.svg"),
            "icons/diffz/file-settings.svg" => include_bytes!("../icons/file-settings.svg"),
            "icons/diffz/file-lock.svg" => include_bytes!("../icons/file-lock.svg"),
            _ => return None,
        })
    }
}
impl From<AppIcon> for Icon {
    fn from(icon: AppIcon) -> Icon {
        Icon::default().path(icon.path())
    }
}
/// Asset source for the app: diffz's icons win, gpui-kit's fill in the rest.
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = AppIcon::bytes(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_app_icon_resolves_to_an_svg() {
        for icon in AppIcon::ALL {
            let bytes = Assets
                .load(icon.path())
                .unwrap()
                .unwrap_or_else(|| panic!("{}", icon.path()));
            assert!(bytes.starts_with(b"<svg"), "{}", icon.path());
        }
    }
    #[test]
    fn kit_icons_still_load() {
        assert!(Assets.load("icons/check.svg").unwrap().is_some());
    }

    #[test]
    fn file_paths_select_editor_file_identities() {
        for (expected, paths) in [
            (
                "icons/diffz/file-rust.svg",
                ["src/lib.rs", "Cargo.toml", "Cargo.lock"].as_slice(),
            ),
            (
                "icons/diffz/file-javascript.svg",
                ["src/lib.js", "src/lib.mjs", "src/lib.cjs"].as_slice(),
            ),
            ("icons/diffz/file-react.svg", ["src/App.jsx"].as_slice()),
            (
                "icons/diffz/file-typescript.svg",
                [
                    "src/lib.ts",
                    "src/lib.mts",
                    "src/lib.cts",
                    "tsconfig.json",
                    "tsconfig.base.JSON",
                ]
                .as_slice(),
            ),
            ("icons/diffz/file-react-ts.svg", ["src/App.tsx"].as_slice()),
            ("icons/diffz/file-json.svg", ["src/data.json"].as_slice()),
            ("icons/diffz/file-python.svg", ["src/main.py"].as_slice()),
            (
                "icons/diffz/file-go.svg",
                ["cmd/main.go", "go.mod", "go.sum"].as_slice(),
            ),
            (
                "icons/diffz/file-c.svg",
                ["src/main.c", "include/main.h"].as_slice(),
            ),
            (
                "icons/diffz/file-cpp.svg",
                ["src/main.cpp", "src/main.cc", "include/main.hpp"].as_slice(),
            ),
            ("icons/diffz/file-java.svg", ["src/Main.java"].as_slice()),
            (
                "icons/diffz/file-ruby.svg",
                ["lib/app.rb", "Gemfile", "Gemfile.lock", "Rakefile"].as_slice(),
            ),
            (
                "icons/diffz/file-shell.svg",
                [
                    "scripts/build.sh",
                    "scripts/build.bash",
                    "scripts/build.zsh",
                    "packaging/arch/PKGBUILD",
                ]
                .as_slice(),
            ),
            ("icons/diffz/file-toml.svg", ["config/app.toml"].as_slice()),
            (
                "icons/diffz/file-yaml.svg",
                ["config/app.yaml", "config/app.yml"].as_slice(),
            ),
            (
                "icons/diffz/file-html.svg",
                ["web/index.html", "web/index.htm"].as_slice(),
            ),
            ("icons/diffz/file-css.svg", ["web/style.css"].as_slice()),
            (
                "icons/diffz/file-elixir.svg",
                ["lib/app.ex", "lib/app.exs", "lib/app.heex", "mix.exs"].as_slice(),
            ),
            (
                "icons/diffz/file-gleam.svg",
                ["src/app.gleam", "gleam.toml"].as_slice(),
            ),
            (
                "icons/diffz/file-markdown.svg",
                [
                    "docs/guide.md",
                    "docs/guide.markdown",
                    "docs/guide.mdx",
                    "README",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-handlebars.svg",
                [
                    "templates/page.hbs",
                    "templates/page.handlebars",
                    "templates/page.mustache",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-image.svg",
                [
                    "assets/logo.png",
                    "assets/logo.jpg",
                    "assets/logo.jpeg",
                    "assets/logo.gif",
                    "assets/logo.svg",
                    "assets/logo.webp",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-diff.svg",
                ["changes.patch", "changes.diff"].as_slice(),
            ),
            (
                "icons/diffz/file-database.svg",
                ["data/query.sql", "data/app.sqlite"].as_slice(),
            ),
            ("icons/diffz/file-pdf.svg", ["docs/guide.pdf"].as_slice()),
            ("icons/diffz/file-xml.svg", ["config/schema.xml"].as_slice()),
            (
                "icons/diffz/file-archive.svg",
                ["releases/app.zip", "releases/app.tar.gz"].as_slice(),
            ),
            (
                "icons/diffz/file-audio.svg",
                ["media/song.mp3", "media/song.wav"].as_slice(),
            ),
            (
                "icons/diffz/file-video.svg",
                ["media/demo.mp4", "media/demo.webm"].as_slice(),
            ),
            (
                "icons/diffz/file-font.svg",
                ["fonts/Inter.ttf", "fonts/Inter.woff2"].as_slice(),
            ),
            (
                "icons/diffz/file-license.svg",
                ["LICENSE", "LICENSE.txt", "COPYING", "COPYING.md"].as_slice(),
            ),
            (
                "icons/diffz/file-makefile.svg",
                ["Makefile", "GNUmakefile"].as_slice(),
            ),
            (
                "icons/diffz/file-docker.svg",
                ["Dockerfile", "Dockerfile.dev", "Containerfile.prod"].as_slice(),
            ),
            (
                "icons/diffz/file-git.svg",
                [".gitignore", ".gitattributes", ".gitmodules"].as_slice(),
            ),
            (
                "icons/diffz/file-github-actions.svg",
                [".github/workflows/ci.yml", ".github/workflows/RELEASE.YAML"].as_slice(),
            ),
            (
                "icons/diffz/file-npm.svg",
                ["package.json", "package-lock.json", "npm-shrinkwrap.json"].as_slice(),
            ),
            (
                "icons/diffz/file-settings.svg",
                [".env", ".env.local", ".editorconfig"].as_slice(),
            ),
            ("icons/diffz/file-lock.svg", ["tmp/session.lock"].as_slice()),
            (
                "icons/diffz/file.svg",
                ["src/example", "src/example.bin"].as_slice(),
            ),
        ] {
            for path in paths {
                assert_eq!(
                    AppIcon::for_path(path).path(),
                    expected,
                    "unexpected icon for {path}"
                );
            }
        }
    }
}
