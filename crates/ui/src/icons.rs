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
    FileCsharp,
    FileFsharp,
    FileVisualBasic,
    FileVisualStudio,
    FileObjectiveC,
    FileSwift,
    FileKotlin,
    FileScala,
    FilePhp,
    FileLua,
    FilePerl,
    FileR,
    FileDart,
    FileHaskell,
    FileZig,
    FileNim,
    FileOcaml,
    FileErlang,
    FileClojure,
    FileVue,
    FileSvelte,
    FileAstro,
    FileGraphql,
    FileProto,
    FileTerraform,
    FileHcl,
    FilePrisma,
    FileJupyter,
    FileGradle,
    FileMaven,
    FileCmake,
    FileMeson,
    FileNix,
    FileAsciidoc,
}
impl AppIcon {
    pub const ALL: [AppIcon; 77] = [
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
        AppIcon::FileCsharp,
        AppIcon::FileFsharp,
        AppIcon::FileVisualBasic,
        AppIcon::FileVisualStudio,
        AppIcon::FileObjectiveC,
        AppIcon::FileSwift,
        AppIcon::FileKotlin,
        AppIcon::FileScala,
        AppIcon::FilePhp,
        AppIcon::FileLua,
        AppIcon::FilePerl,
        AppIcon::FileR,
        AppIcon::FileDart,
        AppIcon::FileHaskell,
        AppIcon::FileZig,
        AppIcon::FileNim,
        AppIcon::FileOcaml,
        AppIcon::FileErlang,
        AppIcon::FileClojure,
        AppIcon::FileVue,
        AppIcon::FileSvelte,
        AppIcon::FileAstro,
        AppIcon::FileGraphql,
        AppIcon::FileProto,
        AppIcon::FileTerraform,
        AppIcon::FileHcl,
        AppIcon::FilePrisma,
        AppIcon::FileJupyter,
        AppIcon::FileGradle,
        AppIcon::FileMaven,
        AppIcon::FileCmake,
        AppIcon::FileMeson,
        AppIcon::FileNix,
        AppIcon::FileAsciidoc,
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
        let is_requirements_variant = basename
            .strip_prefix("requirements-")
            .or_else(|| basename.strip_prefix("requirements."))
            .is_some_and(|suffix| {
                suffix
                    .strip_suffix(".txt")
                    .or_else(|| suffix.strip_suffix(".in"))
                    .is_some_and(|name| !name.is_empty())
            });
        if matches!(
            basename,
            "pyproject.toml"
                | "pipfile"
                | "pipfile.lock"
                | "poetry.lock"
                | "requirements"
                | "requirements.txt"
                | "requirements.in"
        ) || is_requirements_variant
        {
            return Self::FilePython;
        }
        if is_name_variant("gemfile") || is_name_variant("rakefile") {
            return Self::FileRuby;
        }
        if matches!(basename, "mix.exs" | "mix.lock") {
            return Self::FileElixir;
        }
        if matches!(basename, "rebar.config" | "rebar.lock") {
            return Self::FileErlang;
        }
        if basename.ends_with(".rproj") {
            return Self::FileR;
        }
        if matches!(basename, "pubspec.yaml" | "pubspec.lock") {
            return Self::FileDart;
        }
        if matches!(basename, "mvnw" | "mvnw.cmd") {
            return Self::FileMaven;
        }
        if matches!(
            basename,
            "cabal.project" | "stack.yaml" | "package.yaml" | "flake.lock"
        ) || basename.starts_with("cabal.project.")
            || basename.starts_with("stack.yaml.")
        {
            return if basename == "flake.lock" {
                Self::FileNix
            } else {
                Self::FileHaskell
            };
        }
        if basename == ".terraform.lock.hcl"
            || basename.ends_with(".tf.json")
            || basename.ends_with(".tfvars.json")
            || basename.ends_with(".tfstate.backup")
        {
            return Self::FileTerraform;
        }
        if basename.ends_with(".csproj") {
            return Self::FileCsharp;
        }
        if basename.ends_with(".sln") || basename.ends_with(".slnf") || basename.ends_with(".slnx")
        {
            return Self::FileVisualStudio;
        }
        if basename.ends_with(".fsproj") {
            return Self::FileFsharp;
        }
        if basename.ends_with(".vbproj") {
            return Self::FileVisualBasic;
        }
        if basename.ends_with(".vcxproj") {
            return Self::FileCpp;
        }
        if basename == "gleam.toml" {
            return Self::FileGleam;
        }
        if matches!(
            basename,
            "build.gradle"
                | "build.gradle.kts"
                | "settings.gradle"
                | "settings.gradle.kts"
                | "gradle.properties"
                | "gradlew"
                | "gradlew.bat"
                | "gradle-wrapper.properties"
        ) {
            return Self::FileGradle;
        }
        if matches!(basename, "pom.xml" | "maven.config" | "jvm.config") {
            return Self::FileMaven;
        }
        if matches!(
            basename,
            "cmakelists.txt" | "cmakecache.txt" | "cmakepresets.json"
        ) {
            return Self::FileCmake;
        }
        if matches!(
            basename,
            "meson.build" | "meson_options.txt" | "meson.options"
        ) {
            return Self::FileMeson;
        }
        if matches!(
            basename,
            ".graphqlconfig" | "prisma.yml" | "prisma.config.ts"
        ) {
            return if basename == ".graphqlconfig" {
                Self::FileGraphql
            } else {
                Self::FilePrisma
            };
        }
        if basename == "pkgbuild" {
            return Self::FileShell;
        }
        if basename.starts_with(".env") || basename == ".editorconfig" {
            return Self::FileSettings;
        }

        match extension {
            Some("rs") => Self::FileRust,
            Some("cs" | "csx" | "csharp") => Self::FileCsharp,
            Some("fs" | "fsx" | "fsi" | "fsscript") => Self::FileFsharp,
            Some("vb" | "vbs" | "vbproj" | "bas" | "vba") => Self::FileVisualBasic,
            Some("m" | "mm") => Self::FileObjectiveC,
            Some("swift" | "xcplayground") => Self::FileSwift,
            Some("kt" | "kts") => Self::FileKotlin,
            Some("scala" | "sc") => Self::FileScala,
            Some("php" | "php4" | "php5" | "phtml" | "ctp") => Self::FilePhp,
            Some("lua") => Self::FileLua,
            Some("pl" | "pm" | "raku" | "pod" | "psgi" | "t") => Self::FilePerl,
            Some("r" | "rmd" | "rhistory" | "rprofile" | "rt") => Self::FileR,
            Some("dart") => Self::FileDart,
            Some("hs" | "lhs") => Self::FileHaskell,
            Some("cabal") => Self::FileHaskell,
            Some("zig" | "zon") => Self::FileZig,
            Some("nim" | "nimble") => Self::FileNim,
            Some("ml" | "mli" | "cmx") => Self::FileOcaml,
            Some("erl" | "hrl") => Self::FileErlang,
            Some("clj" | "cljs" | "cljc" | "cljx" | "clojure" | "edn") => Self::FileClojure,
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
            Some("vue") => Self::FileVue,
            Some("svelte") => Self::FileSvelte,
            Some("astro") => Self::FileAstro,
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
            Some("graphql" | "gql") => Self::FileGraphql,
            Some("proto") => Self::FileProto,
            Some("tf" | "tfvars" | "tfstate" | "tfbackend") => Self::FileTerraform,
            Some("hcl") => Self::FileHcl,
            Some("prisma") => Self::FilePrisma,
            Some("ipynb") => Self::FileJupyter,
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
            Some("gradle") => Self::FileGradle,
            Some("cmake") => Self::FileCmake,
            Some("wrap") => Self::FileMeson,
            Some("nix") => Self::FileNix,
            Some("ad" | "adoc" | "asciidoc") => Self::FileAsciidoc,
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
            AppIcon::FileCsharp => "icons/diffz/file-csharp.svg",
            AppIcon::FileFsharp => "icons/diffz/file-fsharp.svg",
            AppIcon::FileVisualBasic => "icons/diffz/file-visual-basic.svg",
            AppIcon::FileVisualStudio => "icons/diffz/file-visual-studio.svg",
            AppIcon::FileObjectiveC => "icons/diffz/file-objective-c.svg",
            AppIcon::FileSwift => "icons/diffz/file-swift.svg",
            AppIcon::FileKotlin => "icons/diffz/file-kotlin.svg",
            AppIcon::FileScala => "icons/diffz/file-scala.svg",
            AppIcon::FilePhp => "icons/diffz/file-php.svg",
            AppIcon::FileLua => "icons/diffz/file-lua.svg",
            AppIcon::FilePerl => "icons/diffz/file-perl.svg",
            AppIcon::FileR => "icons/diffz/file-r.svg",
            AppIcon::FileDart => "icons/diffz/file-dart.svg",
            AppIcon::FileHaskell => "icons/diffz/file-haskell.svg",
            AppIcon::FileZig => "icons/diffz/file-zig.svg",
            AppIcon::FileNim => "icons/diffz/file-nim.svg",
            AppIcon::FileOcaml => "icons/diffz/file-ocaml.svg",
            AppIcon::FileErlang => "icons/diffz/file-erlang.svg",
            AppIcon::FileClojure => "icons/diffz/file-clojure.svg",
            AppIcon::FileVue => "icons/diffz/file-vue.svg",
            AppIcon::FileSvelte => "icons/diffz/file-svelte.svg",
            AppIcon::FileAstro => "icons/diffz/file-astro.svg",
            AppIcon::FileGraphql => "icons/diffz/file-graphql.svg",
            AppIcon::FileProto => "icons/diffz/file-proto.svg",
            AppIcon::FileTerraform => "icons/diffz/file-terraform.svg",
            AppIcon::FileHcl => "icons/diffz/file-hcl.svg",
            AppIcon::FilePrisma => "icons/diffz/file-prisma.svg",
            AppIcon::FileJupyter => "icons/diffz/file-jupyter.svg",
            AppIcon::FileGradle => "icons/diffz/file-gradle.svg",
            AppIcon::FileMaven => "icons/diffz/file-maven.svg",
            AppIcon::FileCmake => "icons/diffz/file-cmake.svg",
            AppIcon::FileMeson => "icons/diffz/file-meson.svg",
            AppIcon::FileNix => "icons/diffz/file-nix.svg",
            AppIcon::FileAsciidoc => "icons/diffz/file-asciidoc.svg",
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
            "icons/diffz/file-csharp.svg" => include_bytes!("../icons/file-csharp.svg"),
            "icons/diffz/file-fsharp.svg" => include_bytes!("../icons/file-fsharp.svg"),
            "icons/diffz/file-visual-basic.svg" => {
                include_bytes!("../icons/file-visual-basic.svg")
            }
            "icons/diffz/file-visual-studio.svg" => {
                include_bytes!("../icons/file-visual-studio.svg")
            }
            "icons/diffz/file-objective-c.svg" => include_bytes!("../icons/file-objective-c.svg"),
            "icons/diffz/file-swift.svg" => include_bytes!("../icons/file-swift.svg"),
            "icons/diffz/file-kotlin.svg" => include_bytes!("../icons/file-kotlin.svg"),
            "icons/diffz/file-scala.svg" => include_bytes!("../icons/file-scala.svg"),
            "icons/diffz/file-php.svg" => include_bytes!("../icons/file-php.svg"),
            "icons/diffz/file-lua.svg" => include_bytes!("../icons/file-lua.svg"),
            "icons/diffz/file-perl.svg" => include_bytes!("../icons/file-perl.svg"),
            "icons/diffz/file-r.svg" => include_bytes!("../icons/file-r.svg"),
            "icons/diffz/file-dart.svg" => include_bytes!("../icons/file-dart.svg"),
            "icons/diffz/file-haskell.svg" => include_bytes!("../icons/file-haskell.svg"),
            "icons/diffz/file-zig.svg" => include_bytes!("../icons/file-zig.svg"),
            "icons/diffz/file-nim.svg" => include_bytes!("../icons/file-nim.svg"),
            "icons/diffz/file-ocaml.svg" => include_bytes!("../icons/file-ocaml.svg"),
            "icons/diffz/file-erlang.svg" => include_bytes!("../icons/file-erlang.svg"),
            "icons/diffz/file-clojure.svg" => include_bytes!("../icons/file-clojure.svg"),
            "icons/diffz/file-vue.svg" => include_bytes!("../icons/file-vue.svg"),
            "icons/diffz/file-svelte.svg" => include_bytes!("../icons/file-svelte.svg"),
            "icons/diffz/file-astro.svg" => include_bytes!("../icons/file-astro.svg"),
            "icons/diffz/file-graphql.svg" => include_bytes!("../icons/file-graphql.svg"),
            "icons/diffz/file-proto.svg" => include_bytes!("../icons/file-proto.svg"),
            "icons/diffz/file-terraform.svg" => include_bytes!("../icons/file-terraform.svg"),
            "icons/diffz/file-hcl.svg" => include_bytes!("../icons/file-hcl.svg"),
            "icons/diffz/file-prisma.svg" => include_bytes!("../icons/file-prisma.svg"),
            "icons/diffz/file-jupyter.svg" => include_bytes!("../icons/file-jupyter.svg"),
            "icons/diffz/file-gradle.svg" => include_bytes!("../icons/file-gradle.svg"),
            "icons/diffz/file-maven.svg" => include_bytes!("../icons/file-maven.svg"),
            "icons/diffz/file-cmake.svg" => include_bytes!("../icons/file-cmake.svg"),
            "icons/diffz/file-meson.svg" => include_bytes!("../icons/file-meson.svg"),
            "icons/diffz/file-nix.svg" => include_bytes!("../icons/file-nix.svg"),
            "icons/diffz/file-asciidoc.svg" => include_bytes!("../icons/file-asciidoc.svg"),
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
                "icons/diffz/file-terraform.svg",
                [
                    "infra/.terraform.lock.hcl",
                    "infra/config.tf.json",
                    "infra/variables.tfvars.json",
                    "infra/state.tfstate.backup",
                ]
                .as_slice(),
            ),
            ("icons/diffz/file-csharp.svg", ["src/App.csproj"].as_slice()),
            (
                "icons/diffz/file-visual-studio.svg",
                ["src/App.sln", "src/App.slnf", "src/App.slnx"].as_slice(),
            ),
            ("icons/diffz/file-fsharp.svg", ["src/App.fsproj"].as_slice()),
            (
                "icons/diffz/file-visual-basic.svg",
                ["src/App.vbproj"].as_slice(),
            ),
            ("icons/diffz/file-cpp.svg", ["src/App.vcxproj"].as_slice()),
            (
                "icons/diffz/file-maven.svg",
                ["mvnw", "mvnw.cmd"].as_slice(),
            ),
            (
                "icons/diffz/file-dart.svg",
                ["pubspec.yaml", "pubspec.lock"].as_slice(),
            ),
            (
                "icons/diffz/file-r.svg",
                ["analysis/report.rproj"].as_slice(),
            ),
            (
                "icons/diffz/file-erlang.svg",
                ["rebar.config", "rebar.lock"].as_slice(),
            ),
            (
                "icons/diffz/file-haskell.svg",
                [
                    "cabal.project",
                    "cabal.project.local",
                    "stack.yaml",
                    "stack.yaml.lock",
                    "package.yaml",
                    "package.cabal",
                ]
                .as_slice(),
            ),
            ("icons/diffz/file-nix.svg", ["flake.lock"].as_slice()),
            ("icons/diffz/file-elixir.svg", ["mix.lock"].as_slice()),
            (
                "icons/diffz/file-python.svg",
                [
                    "pyproject.toml",
                    "Pipfile",
                    "Pipfile.lock",
                    "poetry.lock",
                    "requirements.txt",
                    "requirements-dev.txt",
                    "requirements",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-csharp.svg",
                ["src/Program.cs", "src/Program.csx"].as_slice(),
            ),
            (
                "icons/diffz/file-fsharp.svg",
                ["src/Program.fs", "src/Program.fsx"].as_slice(),
            ),
            (
                "icons/diffz/file-visual-basic.svg",
                ["src/Module.vb"].as_slice(),
            ),
            (
                "icons/diffz/file-objective-c.svg",
                ["Sources/App.m", "Sources/App.mm"].as_slice(),
            ),
            (
                "icons/diffz/file-swift.svg",
                ["Sources/App.swift"].as_slice(),
            ),
            (
                "icons/diffz/file-kotlin.svg",
                ["src/Main.kt", "src/Main.kts"].as_slice(),
            ),
            (
                "icons/diffz/file-scala.svg",
                ["src/Main.scala", "src/Build.sc"].as_slice(),
            ),
            ("icons/diffz/file-php.svg", ["public/index.php"].as_slice()),
            ("icons/diffz/file-lua.svg", ["scripts/init.lua"].as_slice()),
            (
                "icons/diffz/file-perl.svg",
                ["scripts/build.pl", "lib/Module.pm"].as_slice(),
            ),
            (
                "icons/diffz/file-r.svg",
                ["analysis/report.r", "analysis/report.Rmd"].as_slice(),
            ),
            ("icons/diffz/file-dart.svg", ["lib/main.dart"].as_slice()),
            (
                "icons/diffz/file-haskell.svg",
                ["src/Main.hs", "src/Main.lhs"].as_slice(),
            ),
            ("icons/diffz/file-zig.svg", ["src/main.zig"].as_slice()),
            ("icons/diffz/file-nim.svg", ["src/main.nim"].as_slice()),
            (
                "icons/diffz/file-ocaml.svg",
                ["lib/main.ml", "lib/main.mli"].as_slice(),
            ),
            (
                "icons/diffz/file-erlang.svg",
                ["src/app.erl", "src/app.hrl"].as_slice(),
            ),
            (
                "icons/diffz/file-clojure.svg",
                [
                    "src/app.clj",
                    "src/app.cljs",
                    "src/app.cljc",
                    "data/app.edn",
                ]
                .as_slice(),
            ),
            ("icons/diffz/file-vue.svg", ["web/App.vue"].as_slice()),
            ("icons/diffz/file-svelte.svg", ["web/App.svelte"].as_slice()),
            ("icons/diffz/file-astro.svg", ["web/index.astro"].as_slice()),
            (
                "icons/diffz/file-graphql.svg",
                ["schema.graphql", "schema.gql", ".graphqlconfig"].as_slice(),
            ),
            (
                "icons/diffz/file-proto.svg",
                ["api/service.proto"].as_slice(),
            ),
            (
                "icons/diffz/file-terraform.svg",
                ["infra/main.tf", "infra/variables.tfvars"].as_slice(),
            ),
            ("icons/diffz/file-hcl.svg", ["infra/config.hcl"].as_slice()),
            (
                "icons/diffz/file-prisma.svg",
                ["db/schema.prisma", "prisma.yml", "prisma.config.ts"].as_slice(),
            ),
            (
                "icons/diffz/file-jupyter.svg",
                ["notebooks/analysis.ipynb"].as_slice(),
            ),
            (
                "icons/diffz/file-gradle.svg",
                [
                    "build.gradle",
                    "build.gradle.kts",
                    "settings.gradle",
                    "settings.gradle.kts",
                    "gradle.properties",
                    "gradlew",
                    "gradlew.bat",
                    "gradle-wrapper.properties",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-maven.svg",
                ["pom.xml", "maven.config", "jvm.config"].as_slice(),
            ),
            (
                "icons/diffz/file-cmake.svg",
                [
                    "CMakeLists.txt",
                    "CMakeCache.txt",
                    "CMakePresets.json",
                    "build/tool.cmake",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-meson.svg",
                [
                    "meson.build",
                    "meson_options.txt",
                    "meson.options",
                    "wrap/project.wrap",
                ]
                .as_slice(),
            ),
            (
                "icons/diffz/file-nix.svg",
                ["flake.nix", "shell.nix"].as_slice(),
            ),
            (
                "icons/diffz/file-asciidoc.svg",
                ["docs/guide.ad", "docs/guide.adoc", "docs/guide.asciidoc"].as_slice(),
            ),
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
        assert_eq!(
            AppIcon::for_path("config/requirements.yml").path(),
            "icons/diffz/file-yaml.svg"
        );
        assert_eq!(
            AppIcon::for_path("config/requirements.json").path(),
            "icons/diffz/file-json.svg"
        );
    }
}
