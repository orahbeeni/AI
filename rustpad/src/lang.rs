//! Language definitions: file association, comment syntax, keyword sets.
//! Pure data; `lexer.rs` interprets it.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum Style {
    Default,
    Comment,
    DocComment,
    String,
    Char,
    Number,
    Keyword,
    Type,
    Literal,
    Builtin,
    Function,
    Operator,
    Preproc,
    Variable,
    Tag,
    Attr,
    Entity,
    Heading,
    Bold,
    Italic,
    Link,
    Code,
    Quote,
    Added,
    Removed,
    Hunk,
    Section,
    Key,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Generic,
    Markup,
    Markdown,
    Diff,
    /// `key = value` / `key: value` files: ini, toml, properties, yaml.
    KeyValue,
    /// SAP Commerce ImpEx import scripts.
    Impex,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FoldMode {
    None,
    Braces,
    Indent,
    Tags,
}

pub mod flag {
    pub const CASE_INSENSITIVE: u32 = 1;
    /// `#directive` lines (C family).
    pub const PREPROC: u32 = 1 << 1;
    /// `$name` variables.
    pub const DOLLAR_VAR: u32 = 1 << 2;
    /// `@name` variables.
    pub const AT_VAR: u32 = 1 << 3;
    /// `"""` / `'''` multi-line strings.
    pub const TRIPLE_QUOTE: u32 = 1 << 4;
    /// Backtick strings that may span lines.
    pub const BACKTICK: u32 = 1 << 5;
    /// `'x'` is a char literal rather than a string.
    pub const CHAR_LITERAL: u32 = 1 << 6;
    pub const NESTED_COMMENTS: u32 = 1 << 7;
    /// Identifiers may contain `-` (css, lisp, ...).
    pub const HYPHEN_IDENT: u32 = 1 << 8;
    /// No single-quote strings (apostrophe is something else).
    pub const NO_SQUOTE: u32 = 1 << 9;
    /// `%name` variables (perl hashes / powershell / batch %x%).
    pub const PERCENT_VAR: u32 = 1 << 10;
    /// SQL style: doubled quote escapes the quote.
    pub const DOUBLE_QUOTE_ESCAPE: u32 = 1 << 11;
    /// Strings may continue over a line break.
    pub const MULTILINE_STR: u32 = 1 << 12;
    /// `{Type AS alias}` / `{alias:attribute}` item references (FlexibleSearch).
    pub const BRACE_ITEM: u32 = 1 << 13;
    /// `?param` query parameters.
    pub const QUESTION_VAR: u32 = 1 << 14;
}
use flag::*;

#[derive(Clone, Copy)]
pub struct LangDef {
    pub name: &'static str,
    pub exts: &'static str,
    pub filenames: &'static str,
    pub kind: Kind,
    pub fold: FoldMode,
    pub flags: u32,
    pub line_comment: &'static str,
    pub block_open: &'static str,
    pub block_close: &'static str,
    pub keywords: &'static str,
    pub types: &'static str,
    pub literals: &'static str,
    pub builtins: &'static str,
    /// Shebang interpreter names that select this language.
    pub shebang: &'static str,
    /// Extra characters allowed in identifiers (besides alnum and `_`).
    pub ident_extra: &'static str,
    /// Regex (as text) matching function / symbol definitions for the function list.
    pub symbols: &'static str,
}

const D: LangDef = LangDef {
    name: "",
    exts: "",
    filenames: "",
    kind: Kind::Generic,
    fold: FoldMode::Braces,
    flags: 0,
    line_comment: "",
    block_open: "",
    block_close: "",
    keywords: "",
    types: "",
    literals: "",
    builtins: "",
    shebang: "",
    ident_extra: "",
    symbols: "",
};

const C_COMMON_FLAGS: u32 = PREPROC | CHAR_LITERAL;

pub static LANGS: &[LangDef] = &[
    LangDef { name: "Normal Text", kind: Kind::Plain, fold: FoldMode::None, exts: "txt text log nfo", ..D },
    LangDef {
        name: "C",
        exts: "c h",
        flags: C_COMMON_FLAGS,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "auto break case const continue default do else enum extern for goto if inline register restrict return sizeof static struct switch typedef union volatile while _Alignas _Alignof _Atomic _Generic _Noreturn _Static_assert _Thread_local",
        types: "char double float int long short signed unsigned void bool size_t ssize_t ptrdiff_t int8_t int16_t int32_t int64_t uint8_t uint16_t uint32_t uint64_t intptr_t uintptr_t FILE wchar_t _Bool _Complex",
        literals: "NULL true false EOF stdin stdout stderr",
        symbols: r"^[A-Za-z_][\w\s\*]*?\s+\*?([A-Za-z_]\w*)\s*\([^;]*$",
        ..D
    },
    LangDef {
        name: "C++",
        exts: "cpp cxx cc c++ hpp hxx hh h++ ipp inl tpp ino",
        flags: C_COMMON_FLAGS,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "alignas alignof and and_eq asm auto bitand bitor break case catch class compl concept const consteval constexpr constinit const_cast continue co_await co_return co_yield decltype default delete do dynamic_cast else enum explicit export extern for friend goto if inline mutable namespace new noexcept not not_eq operator or or_eq private protected public register reinterpret_cast requires return sizeof static static_assert static_cast struct switch template this thread_local throw try typedef typeid typename union using virtual volatile while xor xor_eq override final",
        types: "bool char char8_t char16_t char32_t double float int long short signed unsigned void wchar_t size_t string vector map set unordered_map unordered_set array list deque pair tuple optional variant unique_ptr shared_ptr weak_ptr int8_t int16_t int32_t int64_t uint8_t uint16_t uint32_t uint64_t",
        literals: "nullptr NULL true false",
        builtins: "std cout cin cerr endl",
        symbols: r"^[A-Za-z_][\w:<>,\s\*&~]*?\s+\*?&?([A-Za-z_~][\w:]*)\s*\([^;]*$",
        ..D
    },
    LangDef {
        name: "C#",
        exts: "cs csx",
        flags: PREPROC | CHAR_LITERAL,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract as base break case catch checked class const continue default delegate do else enum event explicit extern finally fixed for foreach goto if implicit in interface internal is lock namespace new operator out override params private protected public readonly ref return sealed sizeof stackalloc static struct switch this throw try typeof unchecked unsafe using virtual volatile while var async await yield get set init record where when partial",
        types: "bool byte char decimal double float int long object sbyte short string uint ulong ushort void dynamic String List Dictionary Task",
        literals: "null true false",
        symbols: r"^\s*(?:public|private|protected|internal|static|override|virtual|async|\s)+[\w<>\[\],\s]+\s+([A-Za-z_]\w*)\s*\([^;]*$",
        ..D
    },
    LangDef {
        name: "Java",
        exts: "java",
        flags: CHAR_LITERAL,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract assert break case catch class const continue default do else enum extends final finally for goto if implements import instanceof interface native new package private protected public return static strictfp super switch synchronized this throw throws transient try volatile while var record sealed permits",
        types: "boolean byte char double float int long short void String Object Integer Long Double List Map Set",
        literals: "null true false",
        symbols: r"^\s*(?:public|private|protected|static|final|abstract|synchronized|\s)+[\w<>\[\],\s]+\s+([A-Za-z_]\w*)\s*\([^;]*$",
        ..D
    },
    LangDef {
        name: "JavaScript",
        exts: "js mjs cjs jsx es6",
        flags: BACKTICK | DOUBLE_QUOTE_ESCAPE * 0,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "async await break case catch class const continue debugger default delete do else export extends finally for function if import in instanceof let new of return static super switch this throw try typeof var void while with yield get set from as",
        types: "Array Object String Number Boolean Map Set Promise Symbol Date RegExp Error JSON Math BigInt",
        literals: "null undefined true false NaN Infinity",
        builtins: "console window document require module exports process globalThis",
        ident_extra: "$",
        symbols: r"^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)|^\s*(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:function|\([^)]*\)\s*=>)",
        ..D
    },
    LangDef {
        name: "TypeScript",
        exts: "ts tsx mts cts",
        flags: BACKTICK,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract as async await break case catch class const constructor continue debugger declare default delete do else enum export extends finally for from function get if implements import in infer instanceof interface is keyof let module namespace new of override private protected public readonly return satisfies set static super switch this throw try type typeof var void while with yield",
        types: "any boolean never number object string symbol unknown void bigint Array Record Partial Promise Readonly Pick Omit",
        literals: "null undefined true false NaN Infinity",
        builtins: "console window document require module exports process",
        ident_extra: "$",
        symbols: r"^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)",
        ..D
    },
    LangDef {
        name: "JSON",
        exts: "json jsonc json5 geojson webmanifest",
        filenames: ".eslintrc .babelrc",
        flags: NO_SQUOTE,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        literals: "true false null",
        ..D
    },
    LangDef {
        name: "Rust",
        exts: "rs",
        flags: CHAR_LITERAL | NESTED_COMMENTS,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type union unsafe use where while macro_rules",
        types: "bool char f32 f64 i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize str String Vec Option Result Box Rc Arc HashMap HashSet BTreeMap BTreeSet",
        literals: "true false None Some Ok Err",
        builtins: "println print eprintln eprint format vec panic assert assert_eq assert_ne debug_assert todo unimplemented unreachable write writeln matches dbg include_str env",
        symbols: r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+|async\s+|unsafe\s+|extern\s+(?:\x22[^\x22]*\x22\s+)?)*(?:fn|struct|enum|trait|mod)\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Go",
        exts: "go",
        filenames: "go.mod go.sum",
        flags: BACKTICK | CHAR_LITERAL | MULTILINE_STR,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var",
        types: "bool byte complex64 complex128 error float32 float64 int int8 int16 int32 int64 rune string uint uint8 uint16 uint32 uint64 uintptr any",
        literals: "true false nil iota",
        builtins: "append cap close complex copy delete imag len make new panic print println real recover",
        symbols: r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Python",
        exts: "py pyw pyi pyx gyp gypi bzl",
        filenames: "SConstruct SConscript Pipfile",
        fold: FoldMode::Indent,
        flags: TRIPLE_QUOTE,
        line_comment: "#",
        keywords: "and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case",
        types: "int float str bool list dict set tuple bytes bytearray object type complex frozenset range",
        literals: "True False None self cls __name__ __main__",
        builtins: "print len range open input super isinstance issubclass enumerate zip map filter sorted reversed sum min max abs any all getattr setattr hasattr repr id iter next format vars dir",
        shebang: "python python2 python3",
        symbols: r"^\s*(?:async\s+)?(?:def|class)\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Ruby",
        exts: "rb rbw rake gemspec ru",
        filenames: "Rakefile Gemfile Vagrantfile Guardfile",
        fold: FoldMode::Indent,
        flags: AT_VAR | MULTILINE_STR,
        line_comment: "#",
        keywords: "alias and begin BEGIN break case class def defined? do else elsif end END ensure for if in module next not or redo rescue retry return super then undef unless until when while yield require require_relative include extend attr_accessor attr_reader attr_writer private protected public lambda proc",
        literals: "true false nil self __FILE__ __method__",
        builtins: "puts print p gets raise loop",
        shebang: "ruby",
        symbols: r"^\s*(?:def|class|module)\s+([A-Za-z_][\w.:?!]*)",
        ..D
    },
    LangDef {
        name: "PHP",
        exts: "php php3 php4 php5 phtml phps",
        flags: DOLLAR_VAR,
        line_comment: "// #",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract and array as break callable case catch class clone const continue declare default die do echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile eval exit extends final finally fn for foreach function global goto if implements include include_once instanceof insteadof interface isset list match namespace new or print private protected public readonly require require_once return static switch throw trait try unset use var while xor yield",
        types: "int float string bool array object iterable mixed void never self parent",
        literals: "null true false NULL TRUE FALSE",
        symbols: r"^\s*(?:(?:public|private|protected|static|abstract|final)\s+)*function\s+&?([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Perl",
        exts: "pl pm t pod",
        flags: DOLLAR_VAR | AT_VAR | PERCENT_VAR,
        line_comment: "#",
        keywords: "and cmp continue do else elsif eq for foreach ge gt if last le lt my ne next no not or our package redo require return sub unless until use wantarray while xor local print printf say die warn defined delete exists keys values push pop shift unshift splice reverse sort map grep join split",
        literals: "undef __PACKAGE__ __FILE__ __LINE__",
        shebang: "perl",
        symbols: r"^\s*sub\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Lua",
        exts: "lua",
        fold: FoldMode::Indent,
        line_comment: "--",
        block_open: "--[[",
        block_close: "]]",
        keywords: "and break do else elseif end for function goto if in local not or repeat return then until while",
        literals: "true false nil",
        builtins: "print pairs ipairs require type tostring tonumber setmetatable getmetatable pcall error assert select table string math os io",
        shebang: "lua",
        symbols: r"^\s*(?:local\s+)?function\s+([A-Za-z_][\w.:]*)",
        ..D
    },
    LangDef {
        name: "Bash",
        exts: "sh bash zsh ksh bashrc profile zshrc bash_profile",
        filenames: ".bashrc .bash_profile .profile .zshrc PKGBUILD",
        fold: FoldMode::Braces,
        flags: DOLLAR_VAR | MULTILINE_STR,
        line_comment: "#",
        keywords: "if then elif else fi case esac for select while until do done in function time coproc return break continue exit local export readonly declare typeset unset shift source alias",
        literals: "true false",
        builtins: "echo printf cd pwd ls cat grep sed awk find test read eval exec trap wait kill set sudo mkdir rm cp mv chmod chown head tail sort uniq wc tr cut xargs curl wget tar",
        shebang: "sh bash zsh ksh dash",
        symbols: r"^\s*(?:function\s+)?([A-Za-z_][\w-]*)\s*\(\)\s*\{?|^\s*function\s+([A-Za-z_][\w-]*)",
        ..D
    },
    LangDef {
        name: "PowerShell",
        exts: "ps1 psm1 psd1",
        flags: DOLLAR_VAR | CASE_INSENSITIVE | HYPHEN_IDENT,
        line_comment: "#",
        block_open: "<#",
        block_close: "#>",
        keywords: "begin break catch class continue data do dynamicparam else elseif end exit filter finally for foreach from function if in param process return switch throw trap try until using var while workflow",
        literals: "$true $false $null",
        builtins: "write-host write-output get-childitem get-content set-content get-item set-item new-item remove-item select-object where-object foreach-object sort-object invoke-command invoke-expression",
        symbols: r"(?i)^\s*function\s+([A-Za-z_][\w-]*)",
        ..D
    },
    LangDef {
        name: "Batch",
        exts: "bat cmd",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | PERCENT_VAR,
        line_comment: "rem ::",
        keywords: "if else for in do goto call exit set setlocal endlocal echo pause shift not exist defined errorlevel equ neq lss leq gtr geq start cd md mkdir rd rmdir del copy move ren type title cls color",
        literals: "on off",
        symbols: r"^:([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "SQL",
        exts: "sql ddl dml pls plsql psql",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOUBLE_QUOTE_ESCAPE | NO_SQUOTE * 0,
        line_comment: "-- #",
        block_open: "/*",
        block_close: "*/",
        keywords: "add all alter and as asc begin between by case check column commit constraint create cross database default delete desc distinct drop else end exists foreign from full group having if in index inner insert into is join key left like limit not null offset on or order outer primary references replace returning right rollback select set table then top transaction truncate union unique update values view when where with",
        types: "bigint binary bit blob boolean char clob date datetime decimal double float int integer json money nchar numeric nvarchar real serial smallint text time timestamp tinyint uuid varchar",
        literals: "true false null",
        builtins: "avg coalesce concat count current_date current_timestamp ifnull isnull length lower max min now nullif round substr substring sum trim upper",
        ..D
    },
    LangDef {
        name: "FlexibleSearch",
        exts: "fxs fsq flexsearch flexiblesearch fs",
        fold: FoldMode::Braces,
        flags: CASE_INSENSITIVE | DOUBLE_QUOTE_ESCAPE | BRACE_ITEM | QUESTION_VAR,
        line_comment: "-- #",
        block_open: "/*",
        block_close: "*/",
        keywords: "select from where join left right inner outer full cross on and or not in like is as group by order having union all distinct limit offset asc desc between exists case when then else end localized with top",
        types: "string integer long short byte double float boolean character date",
        literals: "true false null",
        builtins: "count sum max min avg upper lower trim length coalesce ifnull concat cast decode substring round abs mod",
        ident_extra: "-.",
        ..D
    },
    LangDef {
        name: "ImpEx",
        exts: "impex",
        kind: Kind::Impex,
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOUBLE_QUOTE_ESCAPE | MULTILINE_STR | DOLLAR_VAR | NO_SQUOTE,
        line_comment: "#",
        keywords: "insert_update insert update remove",
        literals: "true false null <ignore>",
        ..D
    },
    LangDef {
        name: "HTML",
        exts: "html htm xhtml shtml jsp asp aspx",
        kind: Kind::Markup,
        fold: FoldMode::Tags,
        ..D
    },
    LangDef {
        name: "XML",
        exts: "xml xsl xslt xsd svg rss atom plist csproj vcxproj props targets wsdl xaml resx config manifest ui kml gpx",
        kind: Kind::Markup,
        fold: FoldMode::Tags,
        ..D
    },
    LangDef {
        name: "CSS",
        exts: "css",
        flags: HYPHEN_IDENT | NO_SQUOTE * 0,
        block_open: "/*",
        block_close: "*/",
        literals: "inherit initial unset none auto transparent important",
        ident_extra: "-",
        symbols: r"^([.#]?[A-Za-z_][\w\-\s,.#:>+~\[\]=]*)\s*\{",
        ..D
    },
    LangDef {
        name: "SCSS / Less",
        exts: "scss sass less",
        flags: HYPHEN_IDENT,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "@mixin @include @extend @import @use @forward @function @return @if @else @each @for @while @media @font-face @keyframes",
        literals: "inherit initial unset none auto transparent important",
        ident_extra: "-",
        ..D
    },
    LangDef {
        name: "YAML",
        exts: "yml yaml",
        kind: Kind::KeyValue,
        fold: FoldMode::Indent,
        line_comment: "#",
        literals: "true false null yes no on off True False Null ~",
        ..D
    },
    LangDef {
        name: "TOML",
        exts: "toml",
        filenames: "Cargo.lock Pipfile.lock poetry.lock",
        kind: Kind::KeyValue,
        fold: FoldMode::None,
        line_comment: "#",
        literals: "true false inf nan",
        ..D
    },
    LangDef {
        name: "INI",
        exts: "ini inf cfg conf properties reg desktop editorconfig gitconfig env service",
        filenames: ".gitconfig .editorconfig .env",
        kind: Kind::KeyValue,
        fold: FoldMode::None,
        line_comment: "; #",
        ..D
    },
    LangDef {
        name: "Markdown",
        exts: "md markdown mkd mdown rmd",
        kind: Kind::Markdown,
        fold: FoldMode::None,
        ..D
    },
    LangDef {
        name: "Diff",
        exts: "diff patch rej",
        kind: Kind::Diff,
        fold: FoldMode::None,
        ..D
    },
    LangDef {
        name: "Makefile",
        exts: "mak mk",
        filenames: "Makefile makefile GNUmakefile",
        fold: FoldMode::None,
        flags: DOLLAR_VAR,
        line_comment: "#",
        keywords: "ifeq ifneq ifdef ifndef else endif include -include sinclude define endef export unexport override vpath",
        builtins: "subst patsubst strip findstring filter filter-out sort word wordlist words firstword lastword dir notdir suffix basename addsuffix addprefix join wildcard realpath abspath if or and foreach call value eval origin flavor shell error warning info",
        ident_extra: "-.",
        ..D
    },
    LangDef {
        name: "CMake",
        exts: "cmake",
        filenames: "CMakeLists.txt",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOLLAR_VAR,
        line_comment: "#",
        keywords: "if elseif else endif foreach endforeach while endwhile function endfunction macro endmacro return break continue set unset option include find_package find_library find_path add_executable add_library add_subdirectory add_definitions add_custom_target add_custom_command add_dependencies target_link_libraries target_include_directories target_compile_definitions target_compile_options target_sources install project cmake_minimum_required message list string file configure_file execute_process",
        literals: "TRUE FALSE ON OFF YES NO",
        ..D
    },
    LangDef {
        name: "Dockerfile",
        exts: "dockerfile",
        filenames: "Dockerfile Containerfile",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOLLAR_VAR,
        line_comment: "#",
        keywords: "from run cmd label maintainer expose env add copy entrypoint volume user workdir arg onbuild stopsignal healthcheck shell as",
        ..D
    },
    LangDef {
        name: "Swift",
        exts: "swift",
        flags: PREPROC | NESTED_COMMENTS,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "associatedtype break case catch class continue default defer deinit do else enum extension fallthrough fileprivate for func guard if import in indirect init inout internal is let open operator override private protocol public repeat rethrows return static struct subscript super switch throw throws try typealias var where while async await actor some any",
        types: "Int Int8 Int16 Int32 Int64 UInt Double Float Bool String Character Array Dictionary Set Optional Any AnyObject Void",
        literals: "nil true false self Self",
        symbols: r"^\s*(?:(?:public|private|internal|static|override|final|open|mutating)\s+)*(?:func|class|struct|enum|protocol|extension)\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Kotlin",
        exts: "kt kts",
        flags: CHAR_LITERAL | NESTED_COMMENTS,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract annotation as break by catch class companion const constructor continue crossinline data do else enum expect external final finally for fun get if import in infix init inline inner interface internal is lateinit noinline object open operator out override package private protected public reified return sealed set super suspend tailrec this throw try typealias val var vararg when where while",
        types: "Int Long Short Byte Float Double Boolean Char String Unit Any Nothing Array List Map Set",
        literals: "null true false it",
        symbols: r"^\s*(?:(?:public|private|internal|protected|override|open|abstract|suspend|inline)\s+)*(?:fun|class|object|interface)\s+(?:<[^>]*>\s*)?([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Scala",
        exts: "scala sbt sc",
        flags: NESTED_COMMENTS | TRIPLE_QUOTE | CHAR_LITERAL,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract case catch class def do else extends final finally for forSome if implicit import lazy match new object override package private protected return sealed super this throw trait try type val var while with yield given using enum export then",
        types: "Int Long Short Byte Float Double Boolean Char String Unit Any AnyRef AnyVal Nothing Option List Map Set Seq Vector Array",
        literals: "null true false None Some",
        symbols: r"^\s*(?:(?:private|protected|override|final|implicit|lazy|abstract|sealed|case)\s+)*(?:def|class|object|trait)\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Dart",
        exts: "dart",
        flags: TRIPLE_QUOTE | DOLLAR_VAR * 0,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract as assert async await break case catch class const continue covariant default deferred do dynamic else enum export extends extension external factory final finally for Function get hide if implements import in interface is late library mixin new on operator part required rethrow return set show static super switch sync this throw try typedef var void while with yield",
        types: "int double num String bool List Map Set Future Stream Object dynamic void",
        literals: "null true false",
        ..D
    },
    LangDef {
        name: "Haskell",
        exts: "hs lhs",
        fold: FoldMode::Indent,
        flags: NESTED_COMMENTS | NO_SQUOTE,
        line_comment: "--",
        block_open: "{-",
        block_close: "-}",
        keywords: "case class data default deriving do else foreign if import in infix infixl infixr instance let module newtype of then type where forall qualified as hiding",
        types: "Int Integer Float Double Bool Char String Maybe Either IO Ordering",
        literals: "True False Nothing Just Left Right",
        ident_extra: "'",
        symbols: r"^([a-z_][\w']*)\s*::",
        ..D
    },
    LangDef {
        name: "Lisp / Scheme",
        exts: "lisp lsp el scm ss rkt cl clj cljs edn",
        fold: FoldMode::None,
        flags: HYPHEN_IDENT | NO_SQUOTE | CASE_INSENSITIVE,
        line_comment: ";",
        keywords: "define defun defmacro defvar defparameter defstruct defclass defmethod defgeneric lambda let let* letrec if cond case when unless do loop progn setq set! setf quote begin and or not car cdr cons list append map filter reduce import require provide ns def defn fn recur",
        literals: "t nil #t #f true false",
        ident_extra: "-!?*<>=+/:",
        ..D
    },
    LangDef {
        name: "R",
        exts: "r rdata rds rprofile",
        fold: FoldMode::Braces,
        line_comment: "#",
        keywords: "if else repeat while function for in next break return library require source",
        literals: "TRUE FALSE NULL NA NA_integer_ NA_real_ NA_character_ Inf NaN",
        builtins: "c list vector matrix data.frame print cat paste paste0 length names seq rep apply lapply sapply vapply mapply sum mean median max min",
        ident_extra: ".",
        symbols: r"^\s*([A-Za-z_.][\w.]*)\s*(?:<-|=)\s*function",
        ..D
    },
    LangDef {
        name: "Julia",
        exts: "jl",
        fold: FoldMode::Indent,
        flags: TRIPLE_QUOTE | CHAR_LITERAL | NESTED_COMMENTS,
        line_comment: "#",
        block_open: "#=",
        block_close: "=#",
        keywords: "abstract baremodule begin break catch const continue do else elseif end export finally for function global if import let local macro module mutable primitive quote return struct try type using where while in isa",
        types: "Int Int8 Int16 Int32 Int64 UInt Float32 Float64 Bool Char String Any Nothing Vector Matrix Array Dict Set Tuple",
        literals: "true false nothing missing NaN Inf",
        ident_extra: "!",
        symbols: r"^\s*(?:function|macro|struct)\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "Pascal / Delphi",
        exts: "pas pp dpr dfm lpr",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOUBLE_QUOTE_ESCAPE | NO_SQUOTE * 0,
        line_comment: "//",
        block_open: "{",
        block_close: "}",
        keywords: "and array as asm begin case class const constructor destructor div do downto else end except exports file finalization finally for function goto if implementation in inherited initialization inline interface is label library mod nil not object of on or out packed private procedure program property protected public raise record repeat resourcestring set shl shr then threadvar to try type unit until uses var while with xor",
        types: "integer byte word cardinal longint shortint int64 real double single extended boolean char string pchar pointer",
        literals: "true false nil",
        ..D
    },
    LangDef {
        name: "Visual Basic",
        exts: "vb vbs bas cls frm vba",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOUBLE_QUOTE_ESCAPE | NO_SQUOTE,
        line_comment: "' rem",
        keywords: "addhandler addressof alias and andalso as byref byval call case catch class const continue declare default delegate dim directcast do each else elseif end enum erase error event exit finally for friend function get gettype global gosub goto handles if implements imports in inherits interface is let lib like loop me mod module mustinherit mustoverride mybase myclass namespace narrowing new next not nothing notinheritable notoverridable of on operator option optional or orelse overloads overridable overrides paramarray partial private property protected public raiseevent readonly redim removehandler resume return select set shadows shared static step stop structure sub synclock then throw to try typeof using wend when while widening with withevents writeonly xor",
        types: "boolean byte char date decimal double integer long object sbyte short single string uinteger ulong ushort variant",
        literals: "true false nothing",
        ..D
    },
    LangDef {
        name: "Assembly",
        exts: "asm s nasm inc",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | PREPROC * 0,
        line_comment: "; #",
        keywords: "mov push pop lea add sub mul div imul idiv inc dec and or xor not shl shr sal sar rol ror cmp test jmp je jne jz jnz jg jge jl jle ja jae jb jbe call ret int nop leave enter syscall cdq cqo movzx movsx xchg loop rep section segment global extern db dw dd dq resb resw resd resq equ times org bits default",
        types: "byte word dword qword ptr",
        builtins: "eax ebx ecx edx esi edi esp ebp rax rbx rcx rdx rsi rdi rsp rbp r8 r9 r10 r11 r12 r13 r14 r15 ax bx cx dx al ah bl bh cl ch dl dh xmm0 xmm1 xmm2 xmm3",
        ident_extra: ".$@",
        symbols: r"^([A-Za-z_.][\w.]*):",
        ..D
    },
    LangDef {
        name: "LaTeX",
        exts: "tex sty cls bib",
        fold: FoldMode::None,
        flags: NO_SQUOTE,
        line_comment: "%",
        ident_extra: "\\",
        symbols: r"^\\(?:section|subsection|subsubsection|chapter)\*?\{([^}]*)\}",
        ..D
    },
    LangDef {
        name: "Objective-C",
        exts: "m mm",
        flags: C_COMMON_FLAGS | AT_VAR,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "auto break case const continue default do else enum extern for goto if inline register return sizeof static struct switch typedef union volatile while @interface @implementation @end @property @synthesize @protocol @class @selector @try @catch @finally @throw self super",
        types: "char double float int long short signed unsigned void BOOL id Class SEL IMP NSString NSArray NSDictionary NSObject NSInteger NSUInteger",
        literals: "nil NULL YES NO true false",
        ..D
    },
    LangDef {
        name: "Groovy / Gradle",
        exts: "groovy gradle gvy",
        flags: TRIPLE_QUOTE,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract as assert break case catch class const continue def default do else enum extends final finally for goto if implements import in instanceof interface native new package private protected public return static super switch this throw throws trait try while",
        types: "boolean byte char double float int long short void String Object List Map",
        literals: "null true false",
        ..D
    },
    LangDef {
        name: "D",
        exts: "d di",
        flags: NESTED_COMMENTS | CHAR_LITERAL,
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "abstract alias align asm assert auto body break case cast catch class const continue debug default delegate delete deprecated do else enum export extern final finally for foreach foreach_reverse function goto if immutable import in inout interface invariant is lazy mixin module new nothrow out override package pragma private protected public pure ref return scope shared static struct super switch synchronized template this throw try typeid typeof union unittest version while with",
        types: "bool byte char dchar double float int long real short string ubyte uint ulong ushort void wchar",
        literals: "null true false",
        ..D
    },
    LangDef {
        name: "Zig",
        exts: "zig",
        flags: CHAR_LITERAL,
        line_comment: "//",
        keywords: "addrspace align allowzero and anyframe anytype asm async await break callconv catch comptime const continue defer else enum errdefer error export extern fn for if inline linksection noalias noinline nosuspend opaque or orelse packed pub resume return struct suspend switch test threadlocal try union unreachable usingnamespace var volatile while",
        types: "bool f16 f32 f64 f128 i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize void noreturn type anyerror comptime_int comptime_float c_int c_long",
        literals: "true false null undefined",
        ..D
    },
    LangDef {
        name: "Fortran",
        exts: "f f90 f95 f03 f08 for ftn",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | DOUBLE_QUOTE_ESCAPE,
        line_comment: "!",
        keywords: "allocatable allocate assign associate backspace block call case character close common complex contains continue cycle data deallocate dimension do double else elseif end endif enddo endfile entry equivalence exit external format function goto if implicit include inquire integer intent interface intrinsic logical module namelist nullify only open optional parameter pointer precision print private program public read real recursive result return rewind save select stop subroutine target then type use where while write",
        literals: ".true. .false.",
        ..D
    },
    LangDef {
        name: "Tcl",
        exts: "tcl tk",
        fold: FoldMode::Braces,
        flags: DOLLAR_VAR,
        line_comment: "#",
        keywords: "after append array break catch cd close continue eval exec exit expr file for foreach format gets glob global if incr info join lappend lindex linsert list llength lrange lreplace lsearch lsort namespace open proc puts read regexp regsub rename return scan set split string switch tell time unset uplevel upvar variable vwait while",
        ..D
    },
    LangDef {
        name: "Nim",
        exts: "nim nims nimble",
        fold: FoldMode::Indent,
        flags: TRIPLE_QUOTE | CHAR_LITERAL,
        line_comment: "#",
        keywords: "addr and as asm bind block break case cast concept const continue converter defer discard distinct div do elif else end enum except export finally for from func if import in include interface is isnot iterator let macro method mixin mod nil not notin object of or out proc ptr raise ref return shl shr static template try tuple type using var when while xor yield",
        types: "int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 float float32 float64 bool char string cstring seq array set",
        literals: "true false nil",
        ..D
    },
    LangDef {
        name: "Elixir",
        exts: "ex exs eex",
        fold: FoldMode::Indent,
        flags: TRIPLE_QUOTE | AT_VAR | HYPHEN_IDENT * 0,
        line_comment: "#",
        keywords: "after alias and case catch cond def defdelegate defexception defguard defimpl defmacro defmodule defp defprotocol defstruct do else end fn for if import in not or quote raise receive require rescue try unless unquote use when with",
        literals: "true false nil",
        ident_extra: "?!",
        symbols: r"^\s*defp?\s+([a-z_]\w*[?!]?)",
        ..D
    },
    LangDef {
        name: "Erlang",
        exts: "erl hrl",
        fold: FoldMode::None,
        line_comment: "%",
        keywords: "after and andalso band begin bnot bor bsl bsr bxor case catch cond div end fun if let not of or orelse receive rem try when xor",
        literals: "true false undefined ok error",
        symbols: r"^([a-z]\w*)\s*\(",
        ..D
    },
    LangDef {
        name: "Protobuf",
        exts: "proto",
        line_comment: "//",
        block_open: "/*",
        block_close: "*/",
        keywords: "syntax package import option message enum service rpc returns stream oneof map reserved extensions extend optional required repeated public weak",
        types: "double float int32 int64 uint32 uint64 sint32 sint64 fixed32 fixed64 sfixed32 sfixed64 bool string bytes",
        literals: "true false",
        symbols: r"^\s*(?:message|enum|service|rpc)\s+([A-Za-z_]\w*)",
        ..D
    },
    LangDef {
        name: "GraphQL",
        exts: "graphql gql",
        fold: FoldMode::Braces,
        line_comment: "#",
        keywords: "query mutation subscription fragment on type interface union enum input scalar schema extend directive implements",
        types: "Int Float String Boolean ID",
        literals: "true false null",
        ..D
    },
    LangDef {
        name: "Verilog / VHDL",
        exts: "v sv svh vh vhd vhdl",
        fold: FoldMode::None,
        flags: CASE_INSENSITIVE | PREPROC * 0,
        line_comment: "//  --",
        block_open: "/*",
        block_close: "*/",
        keywords: "always and architecture assign begin block case component constant downto else elsif end entity for function generate generic if in initial inout input is library loop module nand nor not of or others out output package parameter port process procedure reg report return select signal then to type use variable wait when while wire with xor",
        types: "bit bit_vector boolean integer logic natural positive real std_logic std_logic_vector std_ulogic string time unsigned signed",
        literals: "true false",
        ..D
    },
];

pub type LangId = usize;

pub const PLAIN: LangId = 0;

pub struct Compiled {
    pub def: &'static LangDef,
    pub words: HashMap<String, Style>,
    pub line_comments: Vec<&'static str>,
    pub case_insensitive: bool,
}

impl Compiled {
    pub fn has(&self, f: u32) -> bool {
        self.def.flags & f != 0
    }
}

fn compile(def: &'static LangDef) -> Compiled {
    let ci = def.flags & CASE_INSENSITIVE != 0;
    let mut words = HashMap::new();
    let mut add = |list: &'static str, st: Style| {
        for w in list.split_whitespace() {
            let key = if ci { w.to_lowercase() } else { w.to_string() };
            words.entry(key).or_insert(st);
        }
    };
    add(def.keywords, Style::Keyword);
    add(def.types, Style::Type);
    add(def.literals, Style::Literal);
    add(def.builtins, Style::Builtin);
    Compiled {
        def,
        words,
        line_comments: def.line_comment.split_whitespace().collect(),
        case_insensitive: ci,
    }
}

pub fn compiled(id: LangId) -> &'static Compiled {
    static CELL: OnceLock<Vec<Compiled>> = OnceLock::new();
    let all = CELL.get_or_init(|| LANGS.iter().map(compile).collect());
    &all[id.min(all.len() - 1)]
}

pub fn name(id: LangId) -> &'static str {
    LANGS[id.min(LANGS.len() - 1)].name
}

pub fn find_by_name(name: &str) -> Option<LangId> {
    LANGS.iter().position(|l| l.name.eq_ignore_ascii_case(name))
}

/// Pick a language from a file name and (optionally) the first line of content.
pub fn detect(path: Option<&std::path::Path>, first_line: &str) -> LangId {
    if let Some(p) = path {
        let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        for (i, l) in LANGS.iter().enumerate() {
            if l.filenames.split_whitespace().any(|f| f == fname) {
                return i;
            }
        }
        if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            let ext = ext.to_lowercase();
            for (i, l) in LANGS.iter().enumerate().skip(1) {
                if l.exts.split_whitespace().any(|e| e == ext) {
                    return i;
                }
            }
        } else {
            // dotfiles such as ".bashrc" have no extension, match their stem too
            let stem = fname.trim_start_matches('.').to_lowercase();
            for (i, l) in LANGS.iter().enumerate().skip(1) {
                if l.exts.split_whitespace().any(|e| e == stem) {
                    return i;
                }
            }
        }
    }
    if let Some(rest) = first_line.strip_prefix("#!") {
        let mut it = rest.split_whitespace();
        let mut prog = it.next().unwrap_or("");
        if prog.ends_with("/env") || prog == "env" {
            prog = it.next().unwrap_or("");
        }
        let prog = prog.rsplit('/').next().unwrap_or("");
        let base = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        for (i, l) in LANGS.iter().enumerate() {
            if l.shebang.split_whitespace().any(|s| s == prog || s == base) {
                return i;
            }
        }
    }
    if first_line.starts_with("<?xml") {
        return find_by_name("XML").unwrap_or(PLAIN);
    }
    if first_line.starts_with("<?php") {
        return find_by_name("PHP").unwrap_or(PLAIN);
    }
    PLAIN
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects() {
        let n = |l| name(l);
        assert_eq!(n(detect(Some(Path::new("a/b/main.rs")), "")), "Rust");
        assert_eq!(n(detect(Some(Path::new("CMakeLists.txt")), "")), "CMake");
        assert_eq!(n(detect(Some(Path::new("Makefile")), "")), "Makefile");
        assert_eq!(n(detect(Some(Path::new("x.unknownext")), "#!/usr/bin/env python3")), "Python");
        assert_eq!(n(detect(Some(Path::new(".bashrc")), "")), "Bash");
        assert_eq!(n(detect(None, "#!/bin/bash")), "Bash");
        assert_eq!(n(detect(Some(Path::new("notes.txt")), "")), "Normal Text");
    }

    #[test]
    fn no_duplicate_names() {
        let mut names: Vec<_> = LANGS.iter().map(|l| l.name).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
    }
}
