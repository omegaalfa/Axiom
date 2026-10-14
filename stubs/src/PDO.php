<?php
// Runtime declarations exported from extension: PDO

namespace {
    function pdo_drivers(): array {}

    class PDOException extends \RuntimeException implements \Stringable, \Throwable {
        protected  $code = 0;
        public  ?array $errorInfo = NULL;
    }

    class PDO {
        public const PARAM_NULL = 0;
        public const PARAM_BOOL = 5;
        public const PARAM_INT = 1;
        public const PARAM_STR = 2;
        public const PARAM_LOB = 3;
        public const PARAM_STMT = 4;
        public const PARAM_INPUT_OUTPUT = 2147483648;
        public const PARAM_STR_NATL = 1073741824;
        public const PARAM_STR_CHAR = 536870912;
        public const PARAM_EVT_ALLOC = 0;
        public const PARAM_EVT_FREE = 1;
        public const PARAM_EVT_EXEC_PRE = 2;
        public const PARAM_EVT_EXEC_POST = 3;
        public const PARAM_EVT_FETCH_PRE = 4;
        public const PARAM_EVT_FETCH_POST = 5;
        public const PARAM_EVT_NORMALIZE = 6;
        public const FETCH_DEFAULT = 0;
        public const FETCH_LAZY = 1;
        public const FETCH_ASSOC = 2;
        public const FETCH_NUM = 3;
        public const FETCH_BOTH = 4;
        public const FETCH_OBJ = 5;
        public const FETCH_BOUND = 6;
        public const FETCH_COLUMN = 7;
        public const FETCH_CLASS = 8;
        public const FETCH_INTO = 9;
        public const FETCH_FUNC = 10;
        public const FETCH_GROUP = 32;
        public const FETCH_UNIQUE = 64;
        public const FETCH_KEY_PAIR = 12;
        public const FETCH_CLASSTYPE = 128;
        public const FETCH_SERIALIZE = 512;
        public const FETCH_PROPS_LATE = 256;
        public const FETCH_NAMED = 11;
        public const ATTR_AUTOCOMMIT = 0;
        public const ATTR_PREFETCH = 1;
        public const ATTR_TIMEOUT = 2;
        public const ATTR_ERRMODE = 3;
        public const ATTR_SERVER_VERSION = 4;
        public const ATTR_CLIENT_VERSION = 5;
        public const ATTR_SERVER_INFO = 6;
        public const ATTR_CONNECTION_STATUS = 7;
        public const ATTR_CASE = 8;
        public const ATTR_CURSOR_NAME = 9;
        public const ATTR_CURSOR = 10;
        public const ATTR_ORACLE_NULLS = 11;
        public const ATTR_PERSISTENT = 12;
        public const ATTR_STATEMENT_CLASS = 13;
        public const ATTR_FETCH_TABLE_NAMES = 14;
        public const ATTR_FETCH_CATALOG_NAMES = 15;
        public const ATTR_DRIVER_NAME = 16;
        public const ATTR_STRINGIFY_FETCHES = 17;
        public const ATTR_MAX_COLUMN_LEN = 18;
        public const ATTR_EMULATE_PREPARES = 20;
        public const ATTR_DEFAULT_FETCH_MODE = 19;
        public const ATTR_DEFAULT_STR_PARAM = 21;
        public const ERRMODE_SILENT = 0;
        public const ERRMODE_WARNING = 1;
        public const ERRMODE_EXCEPTION = 2;
        public const CASE_NATURAL = 0;
        public const CASE_LOWER = 2;
        public const CASE_UPPER = 1;
        public const NULL_NATURAL = 0;
        public const NULL_EMPTY_STRING = 1;
        public const NULL_TO_STRING = 2;
        public const ERR_NONE = '00000';
        public const FETCH_ORI_NEXT = 0;
        public const FETCH_ORI_PRIOR = 1;
        public const FETCH_ORI_FIRST = 2;
        public const FETCH_ORI_LAST = 3;
        public const FETCH_ORI_ABS = 4;
        public const FETCH_ORI_REL = 5;
        public const CURSOR_FWDONLY = 0;
        public const CURSOR_SCROLL = 1;
        public const MYSQL_ATTR_USE_BUFFERED_QUERY = 1000;
        public const MYSQL_ATTR_LOCAL_INFILE = 1001;
        public const MYSQL_ATTR_INIT_COMMAND = 1002;
        public const MYSQL_ATTR_COMPRESS = 1003;
        public const MYSQL_ATTR_DIRECT_QUERY = 20;
        public const MYSQL_ATTR_FOUND_ROWS = 1004;
        public const MYSQL_ATTR_IGNORE_SPACE = 1005;
        public const MYSQL_ATTR_SSL_KEY = 1006;
        public const MYSQL_ATTR_SSL_CERT = 1007;
        public const MYSQL_ATTR_SSL_CA = 1008;
        public const MYSQL_ATTR_SSL_CAPATH = 1009;
        public const MYSQL_ATTR_SSL_CIPHER = 1010;
        public const MYSQL_ATTR_SERVER_PUBLIC_KEY = 1011;
        public const MYSQL_ATTR_MULTI_STATEMENTS = 1012;
        public const MYSQL_ATTR_SSL_VERIFY_SERVER_CERT = 1013;
        public const MYSQL_ATTR_LOCAL_INFILE_DIRECTORY = 1014;
        public function __construct(string $dsn, ?string $username = NULL, ?string $password = NULL, ?array $options = NULL) {}
        public static function connect(string $dsn, ?string $username = NULL, ?string $password = NULL, ?array $options = NULL): static {}
        public function beginTransaction() {}
        public function commit() {}
        public function errorCode() {}
        public function errorInfo() {}
        public function exec(string $statement) {}
        public function getAttribute(int $attribute) {}
        public static function getAvailableDrivers() {}
        public function inTransaction() {}
        public function lastInsertId(?string $name = NULL) {}
        public function prepare(string $query, array $options = array ()) {}
        public function query(string $query, ?int $fetchMode = NULL, mixed ...$fetchModeArgs) {}
        public function quote(string $string, int $type = \PDO::PARAM_STR) {}
        public function rollBack() {}
        public function setAttribute(int $attribute, mixed $value) {}
    }

    class PDOStatement implements \IteratorAggregate, \Traversable {
        public  string $queryString;
        public function bindColumn(string|int $column, mixed &$var, int $type = \PDO::PARAM_STR, int $maxLength = 0, mixed $driverOptions = NULL) {}
        public function bindParam(string|int $param, mixed &$var, int $type = \PDO::PARAM_STR, int $maxLength = 0, mixed $driverOptions = NULL) {}
        public function bindValue(string|int $param, mixed $value, int $type = \PDO::PARAM_STR) {}
        public function closeCursor() {}
        public function columnCount() {}
        public function debugDumpParams() {}
        public function errorCode() {}
        public function errorInfo() {}
        public function execute(?array $params = NULL) {}
        public function fetch(int $mode = \PDO::FETCH_DEFAULT, int $cursorOrientation = \PDO::FETCH_ORI_NEXT, int $cursorOffset = 0) {}
        public function fetchAll(int $mode = \PDO::FETCH_DEFAULT, mixed ...$args) {}
        public function fetchColumn(int $column = 0) {}
        public function fetchObject(?string $class = 'stdClass', array $constructorArgs = array ()) {}
        public function getAttribute(int $name) {}
        public function getColumnMeta(int $column) {}
        public function nextRowset() {}
        public function rowCount() {}
        public function setAttribute(int $attribute, mixed $value) {}
        public function setFetchMode(int $mode, mixed ...$args) {}
        public function getIterator(): \Iterator {}
    }

    final class PDORow {
        public  string $queryString;
    }

}
