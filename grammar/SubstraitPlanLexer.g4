// SPDX-License-Identifier: Apache-2.0

// $antlr-format alignTrailingComments on, columnLimit 150, maxEmptyLinesToKeep 1, reflowComments off, useTab off
// $antlr-format allowShortRulesOnASingleLine on, alignSemicolons ownLine

lexer grammar SubstraitPlanLexer;

// Note: caseInsensitive option is not supported by antlr-rust
// Case insensitivity is manually implemented using character classes [Aa][Bb][Cc]

@lexer::header {
// SPDX-License-Identifier: Apache-2.0
}

@lexer::postinclude {
#ifndef _WIN32
#pragma GCC diagnostic ignored "-Wunused-parameter"
#endif
}

channels { CommentsChannel, DirectiveChannel }

tokens { SPACES }

EXTENSION_SPACE: [Ee][Xx][Tt][Ee][Nn][Ss][Ii][Oo][Nn]'_'[Ss][Pp][Aa][Cc][Ee] -> mode(EXTENSIONS);
FUNCTION: [Ff][Uu][Nn][Cc][Tt][Ii][Oo][Nn];
AS: [Aa][Ss];
NAMED: [Nn][Aa][Mm][Ee][Dd];
SCHEMA: [Ss][Cc][Hh][Ee][Mm][Aa];
RELATION: [Rr][Ee][Ll][Aa][Tt][Ii][Oo][Nn];
PIPELINES: [Pp][Ii][Pp][Ee][Ll][Ii][Nn][Ee][Ss];

COMMON: [Cc][Oo][Mm][Mm][Oo][Nn];
BASE_SCHEMA: [Bb][Aa][Ss][Ee]'_'[Ss][Cc][Hh][Ee][Mm][Aa];
FILTER: [Ff][Ii][Ll][Tt][Ee][Rr];
PROJECTION: [Pp][Rr][Oo][Jj][Ee][Cc][Tt][Ii][Oo][Nn];
EXPRESSION: [Ee][Xx][Pp][Rr][Ee][Ss][Ss][Ii][Oo][Nn];
ADVANCED_EXTENSION: [Aa][Dd][Vv][Aa][Nn][Cc][Ee][Dd]'_'[Ee][Xx][Tt][Ee][Nn][Ss][Ii][Oo][Nn];
GROUPING: [Gg][Rr][Oo][Uu][Pp][Ii][Nn][Gg];
MEASURE: [Mm][Ee][Aa][Ss][Uu][Rr][Ee];
INVOCATION: [Ii][Nn][Vv][Oo][Cc][Aa][Tt][Ii][Oo][Nn];
SORT: [Ss][Oo][Rr][Tt];
BY: [Bb][Yy];
COUNT: [Cc][Oo][Uu][Nn][Tt];
OFFSET: [Oo][Ff][Ff][Ss][Ee][Tt];
TYPE: [Tt][Yy][Pp][Ee];
EMIT: [Ee][Mm][Ii][Tt];

SUBQUERY: [Ss][Uu][Bb][Qq][Uu][Ee][Rr][Yy];
EXISTS: [Ee][Xx][Ii][Ss][Tt][Ss];
UNIQUE: [Uu][Nn][Ii][Qq][Uu][Ee];
IN: [Ii][Nn];
ALL: [Aa][Ll][Ll];
ANY: [Aa][Nn][Yy];
COMPARISON: [Ee][Qq]|[Nn][Ee]|[Ll][Tt]|[Gg][Tt]|[Ll][Ee]|[Gg][Ee];

VIRTUAL_TABLE: [Vv][Ii][Rr][Tt][Uu][Aa][Ll]'_'[Tt][Aa][Bb][Ll][Ee];
LOCAL_FILES: [Ll][Oo][Cc][Aa][Ll]'_'[Ff][Ii][Ll][Ee][Ss];
NAMED_TABLE: [Nn][Aa][Mm][Ee][Dd]'_'[Tt][Aa][Bb][Ll][Ee];
EXTENSION_TABLE: [Ee][Xx][Tt][Ee][Nn][Ss][Ii][Oo][Nn]'_'[Tt][Aa][Bb][Ll][Ee];

SOURCE: [Ss][Oo][Uu][Rr][Cc][Ee];
ROOT: [Rr][Oo][Oo][Tt];
ITEMS: [Ii][Tt][Ee][Mm][Ss];
NAMES: [Nn][Aa][Mm][Ee][Ss];
URI_FILE: [Uu][Rr][Ii]'_'[Ff][Ii][Ll][Ee];
URI_PATH: [Uu][Rr][Ii]'_'[Pp][Aa][Tt][Hh];
URI_PATH_GLOB: [Uu][Rr][Ii]'_'[Pp][Aa][Tt][Hh]'_'[Gg][Ll][Oo][Bb];
URI_FOLDER: [Uu][Rr][Ii]'_'[Ff][Oo][Ll][Dd][Ee][Rr];
PARTITION_INDEX: [Pp][Aa][Rr][Tt][Ii][Tt][Ii][Oo][Nn]'_'[Ii][Nn][Dd][Ee][Xx];
START: [Ss][Tt][Aa][Rr][Tt];
LENGTH: [Ll][Ee][Nn][Gg][Tt][Hh];
ORC: [Oo][Rr][Cc];
PARQUET: [Pp][Aa][Rr][Qq][Uu][Ee][Tt];
NULLVAL: [Nn][Uu][Ll][Ll];
TRUEVAL: [Tt][Rr][Uu][Ee];
FALSEVAL: [Ff][Aa][Ll][Ss][Ee];

LIST: [Ll][Ii][Ss][Tt];
MAP: [Mm][Aa][Pp];
STRUCT: [Ss][Tt][Rr][Uu][Cc][Tt];

ARROW: '->';
COLON: ':';
SEMICOLON: ';';
LEFTBRACE: '{';
RIGHTBRACE: '}';
LEFTPAREN: '(';
RIGHTPAREN: ')';
fragment QUOTE: '"';
COMMA: ',';
PERIOD: '.';
EQUAL: '=';
LEFTBRACKET: '[';
RIGHTBRACKET: ']';
UNDERSCORE: '_';
MINUS: '-';
LEFTANGLEBRACKET: '<';
RIGHTANGLEBRACKET: '>';
QUESTIONMARK: '?';
ATSIGN: '@';

IDENTIFIER
    : [A-Za-z][A-Za-z0-9_$]*
    ;

NUMBER
    : MINUS? [0-9]+ ( PERIOD [0-9]+ )?
    | MINUS? [0-9]+ ( PERIOD [0-9]+ )? 'E' ('+' | MINUS) [0-9]+
    ;

STRING : '"' (ESCAPEDQUOTE | ~["])* '"' ;
fragment ESCAPEDQUOTE : '\\' '"' ;
fragment HEX : [0-9A-Fa-f] ;
fragment DIGIT : [0-9] ;

RAW_LITERAL_SINGLE_BACKTICK : '`' ~[`]+? '`' -> type(STRING) ;
RAW_LITERAL_DOUBLE_BACKTICK : '``' .+? '``' -> type(STRING) ;
RAW_LITERAL_TRIPLE_BACKTICK : '```' .+? '```' -> type(STRING) ;

SINGLE_LINE_COMMENT: '//' ~[\r\n]* (('\r'? '\n') | EOF) -> skip;

SPACES: [ \u000B\t\r\n]+ -> skip;

mode EXTENSIONS;
fragment SCHEME: [A-Za-z]+ ;
fragment HOSTNAME: [A-Za-z0-9-.]+ ;
fragment FILENAME: [A-Za-z0-9-._]+;
fragment PATH: FILENAME ( '/' FILENAME )*;

URI
    : SCHEME ':' ( '//' HOSTNAME '/' )? PATH
    | '/'? PATH
    ;

EXTENSIONS_LEFTBRACE: '{' -> mode(DEFAULT_MODE), type(LEFTBRACE);

EXTENSIONS_SPACES: [ \u000B\t\r\n]+ -> skip;
