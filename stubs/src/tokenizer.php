<?php
// Runtime declarations exported from extension: tokenizer

namespace {
    function token_get_all(string $code, int $flags = 0): array {}

    function token_name(int $id): string {}

    class PhpToken implements \Stringable {
        public  int $id;
        public  string $text;
        public  int $line;
        public  int $pos;
        public static function tokenize(string $code, int $flags = 0): array {}
        public final function __construct(int $id, string $text, int $line = -1, int $pos = -1) {}
        public function is($kind): bool {}
        public function isIgnorable(): bool {}
        public function getTokenName(): ?string {}
        public function __toString(): string {}
    }

    const T_LNUMBER = 260;

    const T_DNUMBER = 261;

    const T_STRING = 262;

    const T_NAME_FULLY_QUALIFIED = 263;

    const T_NAME_RELATIVE = 264;

    const T_NAME_QUALIFIED = 265;

    const T_VARIABLE = 266;

    const T_INLINE_HTML = 267;

    const T_ENCAPSED_AND_WHITESPACE = 268;

    const T_CONSTANT_ENCAPSED_STRING = 269;

    const T_STRING_VARNAME = 270;

    const T_NUM_STRING = 271;

    const T_INCLUDE = 272;

    const T_INCLUDE_ONCE = 273;

    const T_EVAL = 274;

    const T_REQUIRE = 275;

    const T_REQUIRE_ONCE = 276;

    const T_LOGICAL_OR = 277;

    const T_LOGICAL_XOR = 278;

    const T_LOGICAL_AND = 279;

    const T_PRINT = 280;

    const T_YIELD = 281;

    const T_YIELD_FROM = 282;

    const T_INSTANCEOF = 283;

    const T_NEW = 284;

    const T_CLONE = 285;

    const T_EXIT = 286;

    const T_IF = 287;

    const T_ELSEIF = 288;

    const T_ELSE = 289;

    const T_ENDIF = 290;

    const T_ECHO = 291;

    const T_DO = 292;

    const T_WHILE = 293;

    const T_ENDWHILE = 294;

    const T_FOR = 295;

    const T_ENDFOR = 296;

    const T_FOREACH = 297;

    const T_ENDFOREACH = 298;

    const T_DECLARE = 299;

    const T_ENDDECLARE = 300;

    const T_AS = 301;

    const T_SWITCH = 302;

    const T_ENDSWITCH = 303;

    const T_CASE = 304;

    const T_DEFAULT = 305;

    const T_MATCH = 306;

    const T_BREAK = 307;

    const T_CONTINUE = 308;

    const T_GOTO = 309;

    const T_FUNCTION = 310;

    const T_FN = 311;

    const T_CONST = 312;

    const T_RETURN = 313;

    const T_TRY = 314;

    const T_CATCH = 315;

    const T_FINALLY = 316;

    const T_THROW = 317;

    const T_USE = 318;

    const T_INSTEADOF = 319;

    const T_GLOBAL = 320;

    const T_STATIC = 321;

    const T_ABSTRACT = 322;

    const T_FINAL = 323;

    const T_PRIVATE = 324;

    const T_PROTECTED = 325;

    const T_PUBLIC = 326;

    const T_PRIVATE_SET = 327;

    const T_PROTECTED_SET = 328;

    const T_PUBLIC_SET = 329;

    const T_READONLY = 330;

    const T_VAR = 331;

    const T_UNSET = 332;

    const T_ISSET = 333;

    const T_EMPTY = 334;

    const T_HALT_COMPILER = 335;

    const T_CLASS = 336;

    const T_TRAIT = 337;

    const T_INTERFACE = 338;

    const T_ENUM = 339;

    const T_EXTENDS = 340;

    const T_IMPLEMENTS = 341;

    const T_NAMESPACE = 342;

    const T_LIST = 343;

    const T_ARRAY = 344;

    const T_CALLABLE = 345;

    const T_LINE = 346;

    const T_FILE = 347;

    const T_DIR = 348;

    const T_CLASS_C = 349;

    const T_TRAIT_C = 350;

    const T_METHOD_C = 351;

    const T_FUNC_C = 352;

    const T_PROPERTY_C = 353;

    const T_NS_C = 354;

    const T_ATTRIBUTE = 355;

    const T_PLUS_EQUAL = 356;

    const T_MINUS_EQUAL = 357;

    const T_MUL_EQUAL = 358;

    const T_DIV_EQUAL = 359;

    const T_CONCAT_EQUAL = 360;

    const T_MOD_EQUAL = 361;

    const T_AND_EQUAL = 362;

    const T_OR_EQUAL = 363;

    const T_XOR_EQUAL = 364;

    const T_SL_EQUAL = 365;

    const T_SR_EQUAL = 366;

    const T_COALESCE_EQUAL = 367;

    const T_BOOLEAN_OR = 368;

    const T_BOOLEAN_AND = 369;

    const T_IS_EQUAL = 370;

    const T_IS_NOT_EQUAL = 371;

    const T_IS_IDENTICAL = 372;

    const T_IS_NOT_IDENTICAL = 373;

    const T_IS_SMALLER_OR_EQUAL = 374;

    const T_IS_GREATER_OR_EQUAL = 375;

    const T_SPACESHIP = 376;

    const T_SL = 377;

    const T_SR = 378;

    const T_INC = 379;

    const T_DEC = 380;

    const T_INT_CAST = 381;

    const T_DOUBLE_CAST = 382;

    const T_STRING_CAST = 383;

    const T_ARRAY_CAST = 384;

    const T_OBJECT_CAST = 385;

    const T_BOOL_CAST = 386;

    const T_UNSET_CAST = 387;

    const T_VOID_CAST = 388;

    const T_OBJECT_OPERATOR = 389;

    const T_NULLSAFE_OBJECT_OPERATOR = 390;

    const T_DOUBLE_ARROW = 391;

    const T_COMMENT = 392;

    const T_DOC_COMMENT = 393;

    const T_OPEN_TAG = 394;

    const T_OPEN_TAG_WITH_ECHO = 395;

    const T_CLOSE_TAG = 396;

    const T_WHITESPACE = 397;

    const T_START_HEREDOC = 398;

    const T_END_HEREDOC = 399;

    const T_DOLLAR_OPEN_CURLY_BRACES = 400;

    const T_CURLY_OPEN = 401;

    const T_PAAMAYIM_NEKUDOTAYIM = 402;

    const T_NS_SEPARATOR = 403;

    const T_ELLIPSIS = 404;

    const T_COALESCE = 405;

    const T_POW = 406;

    const T_POW_EQUAL = 407;

    const T_PIPE = 408;

    const T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG = 409;

    const T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG = 410;

    const T_BAD_CHARACTER = 411;

    const T_DOUBLE_COLON = 402;

    const TOKEN_PARSE = 1;

}
