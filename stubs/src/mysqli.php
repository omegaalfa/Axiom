<?php
// Runtime declarations exported from extension: mysqli

namespace {
    function mysqli_affected_rows(\mysqli $mysql): string|int {}

    function mysqli_autocommit(\mysqli $mysql, bool $enable): bool {}

    function mysqli_begin_transaction(\mysqli $mysql, int $flags = 0, ?string $name = NULL): bool {}

    function mysqli_change_user(\mysqli $mysql, string $username, string $password, ?string $database): bool {}

    function mysqli_character_set_name(\mysqli $mysql): string {}

    function mysqli_close(\mysqli $mysql): true {}

    function mysqli_commit(\mysqli $mysql, int $flags = 0, ?string $name = NULL): bool {}

    function mysqli_connect(?string $hostname = NULL, ?string $username = NULL, ?string $password = NULL, ?string $database = NULL, ?int $port = NULL, ?string $socket = NULL): \mysqli|false {}

    function mysqli_connect_errno(): int {}

    function mysqli_connect_error(): ?string {}

    function mysqli_data_seek(\mysqli_result $result, int $offset): bool {}

    function mysqli_dump_debug_info(\mysqli $mysql): bool {}

    function mysqli_debug(string $options): true {}

    function mysqli_errno(\mysqli $mysql): int {}

    function mysqli_error(\mysqli $mysql): string {}

    function mysqli_error_list(\mysqli $mysql): array {}

    function mysqli_stmt_execute(\mysqli_stmt $statement, ?array $params = NULL): bool {}

    function mysqli_execute(\mysqli_stmt $statement, ?array $params = NULL): bool {}

    function mysqli_execute_query(\mysqli $mysql, string $query, ?array $params = NULL): \mysqli_result|bool {}

    function mysqli_fetch_field(\mysqli_result $result): object|false {}

    function mysqli_fetch_fields(\mysqli_result $result): array {}

    function mysqli_fetch_field_direct(\mysqli_result $result, int $index): object|false {}

    function mysqli_fetch_lengths(\mysqli_result $result): array|false {}

    function mysqli_fetch_all(\mysqli_result $result, int $mode = \MYSQLI_NUM): array {}

    function mysqli_fetch_array(\mysqli_result $result, int $mode = \MYSQLI_BOTH): array|false|null {}

    function mysqli_fetch_assoc(\mysqli_result $result): array|false|null {}

    function mysqli_fetch_object(\mysqli_result $result, string $class = 'stdClass', array $constructor_args = array ()): object|false|null {}

    function mysqli_fetch_row(\mysqli_result $result): array|false|null {}

    function mysqli_fetch_column(\mysqli_result $result, int $column = 0): string|int|float|false|null {}

    function mysqli_field_count(\mysqli $mysql): int {}

    function mysqli_field_seek(\mysqli_result $result, int $index): true {}

    function mysqli_field_tell(\mysqli_result $result): int {}

    function mysqli_free_result(\mysqli_result $result): void {}

    function mysqli_get_connection_stats(\mysqli $mysql): array {}

    function mysqli_get_client_stats(): array {}

    function mysqli_get_charset(\mysqli $mysql): ?object {}

    function mysqli_get_client_info(?\mysqli $mysql = NULL): string {}

    function mysqli_get_client_version(): int {}

    function mysqli_get_links_stats(): array {}

    function mysqli_get_host_info(\mysqli $mysql): string {}

    function mysqli_get_proto_info(\mysqli $mysql): int {}

    function mysqli_get_server_info(\mysqli $mysql): string {}

    function mysqli_get_server_version(\mysqli $mysql): int {}

    function mysqli_get_warnings(\mysqli $mysql): \mysqli_warning|false {}

    function mysqli_init(): \mysqli|false {}

    function mysqli_info(\mysqli $mysql): ?string {}

    function mysqli_insert_id(\mysqli $mysql): string|int {}

    function mysqli_kill(\mysqli $mysql, int $process_id): bool {}

    function mysqli_more_results(\mysqli $mysql): bool {}

    function mysqli_multi_query(\mysqli $mysql, string $query): bool {}

    function mysqli_next_result(\mysqli $mysql): bool {}

    function mysqli_num_fields(\mysqli_result $result): int {}

    function mysqli_num_rows(\mysqli_result $result): string|int {}

    function mysqli_options(\mysqli $mysql, int $option, $value): bool {}

    function mysqli_set_opt(\mysqli $mysql, int $option, $value): bool {}

    function mysqli_ping(\mysqli $mysql): bool {}

    function mysqli_poll(?array &$read, ?array &$error, array &$reject, int $seconds, int $microseconds = 0): int|false {}

    function mysqli_prepare(\mysqli $mysql, string $query): \mysqli_stmt|false {}

    function mysqli_report(int $flags): true {}

    function mysqli_query(\mysqli $mysql, string $query, int $result_mode = \MYSQLI_STORE_RESULT): \mysqli_result|bool {}

    function mysqli_real_connect(\mysqli $mysql, ?string $hostname = NULL, ?string $username = NULL, ?string $password = NULL, ?string $database = NULL, ?int $port = NULL, ?string $socket = NULL, int $flags = 0): bool {}

    function mysqli_real_escape_string(\mysqli $mysql, string $string): string {}

    function mysqli_escape_string(\mysqli $mysql, string $string): string {}

    function mysqli_real_query(\mysqli $mysql, string $query): bool {}

    function mysqli_reap_async_query(\mysqli $mysql): \mysqli_result|bool {}

    function mysqli_release_savepoint(\mysqli $mysql, string $name): bool {}

    function mysqli_rollback(\mysqli $mysql, int $flags = 0, ?string $name = NULL): bool {}

    function mysqli_savepoint(\mysqli $mysql, string $name): bool {}

    function mysqli_select_db(\mysqli $mysql, string $database): bool {}

    function mysqli_set_charset(\mysqli $mysql, string $charset): bool {}

    function mysqli_stmt_affected_rows(\mysqli_stmt $statement): string|int {}

    function mysqli_stmt_attr_get(\mysqli_stmt $statement, int $attribute): int {}

    function mysqli_stmt_attr_set(\mysqli_stmt $statement, int $attribute, int $value): bool {}

    function mysqli_stmt_bind_param(\mysqli_stmt $statement, string $types, mixed &...$vars): bool {}

    function mysqli_stmt_bind_result(\mysqli_stmt $statement, mixed &...$vars): bool {}

    function mysqli_stmt_close(\mysqli_stmt $statement): true {}

    function mysqli_stmt_data_seek(\mysqli_stmt $statement, int $offset): void {}

    function mysqli_stmt_errno(\mysqli_stmt $statement): int {}

    function mysqli_stmt_error(\mysqli_stmt $statement): string {}

    function mysqli_stmt_error_list(\mysqli_stmt $statement): array {}

    function mysqli_stmt_fetch(\mysqli_stmt $statement): ?bool {}

    function mysqli_stmt_field_count(\mysqli_stmt $statement): int {}

    function mysqli_stmt_free_result(\mysqli_stmt $statement): void {}

    function mysqli_stmt_get_result(\mysqli_stmt $statement): \mysqli_result|false {}

    function mysqli_stmt_get_warnings(\mysqli_stmt $statement): \mysqli_warning|false {}

    function mysqli_stmt_init(\mysqli $mysql): \mysqli_stmt|false {}

    function mysqli_stmt_insert_id(\mysqli_stmt $statement): string|int {}

    function mysqli_stmt_more_results(\mysqli_stmt $statement): bool {}

    function mysqli_stmt_next_result(\mysqli_stmt $statement): bool {}

    function mysqli_stmt_num_rows(\mysqli_stmt $statement): string|int {}

    function mysqli_stmt_param_count(\mysqli_stmt $statement): int {}

    function mysqli_stmt_prepare(\mysqli_stmt $statement, string $query): bool {}

    function mysqli_stmt_reset(\mysqli_stmt $statement): bool {}

    function mysqli_stmt_result_metadata(\mysqli_stmt $statement): \mysqli_result|false {}

    function mysqli_stmt_send_long_data(\mysqli_stmt $statement, int $param_num, string $data): bool {}

    function mysqli_stmt_store_result(\mysqli_stmt $statement): bool {}

    function mysqli_stmt_sqlstate(\mysqli_stmt $statement): string {}

    function mysqli_sqlstate(\mysqli $mysql): string {}

    function mysqli_ssl_set(\mysqli $mysql, ?string $key, ?string $certificate, ?string $ca_certificate, ?string $ca_path, ?string $cipher_algos): true {}

    function mysqli_stat(\mysqli $mysql): string|false {}

    function mysqli_store_result(\mysqli $mysql, int $mode = 0): \mysqli_result|false {}

    function mysqli_thread_id(\mysqli $mysql): int {}

    function mysqli_thread_safe(): bool {}

    function mysqli_use_result(\mysqli $mysql): \mysqli_result|false {}

    function mysqli_warning_count(\mysqli $mysql): int {}

    function mysqli_refresh(\mysqli $mysql, int $flags): bool {}

    final class mysqli_sql_exception extends \RuntimeException implements \Stringable, \Throwable {
        protected  string $sqlstate = '00000';
        public function getSqlState(): string {}
    }

    final class mysqli_driver {
        public  string $client_info;
        public  int $client_version;
        public  int $driver_version;
        public  int $report_mode = 0;
    }

    class mysqli {
        public  string|int $affected_rows;
        public  string $client_info;
        public  int $client_version;
        public  int $connect_errno;
        public  ?string $connect_error;
        public  int $errno;
        public  string $error;
        public  array $error_list;
        public  int $field_count;
        public  string $host_info;
        public  ?string $info;
        public  string|int $insert_id;
        public  string $server_info;
        public  int $server_version;
        public  string $sqlstate;
        public  int $protocol_version;
        public  int $thread_id;
        public  int $warning_count;
        public function __construct(?string $hostname = NULL, ?string $username = NULL, ?string $password = NULL, ?string $database = NULL, ?int $port = NULL, ?string $socket = NULL) {}
        public function autocommit(bool $enable) {}
        public function begin_transaction(int $flags = 0, ?string $name = NULL) {}
        public function change_user(string $username, string $password, ?string $database) {}
        public function character_set_name() {}
        public function close() {}
        public function commit(int $flags = 0, ?string $name = NULL) {}
        public function connect(?string $hostname = NULL, ?string $username = NULL, ?string $password = NULL, ?string $database = NULL, ?int $port = NULL, ?string $socket = NULL) {}
        public function dump_debug_info() {}
        public function debug(string $options) {}
        public function get_charset() {}
        public function execute_query(string $query, ?array $params = NULL): \mysqli_result|bool {}
        public function get_client_info() {}
        public function get_connection_stats() {}
        public function get_server_info() {}
        public function get_warnings() {}
        public function init() {}
        public function kill(int $process_id) {}
        public function multi_query(string $query) {}
        public function more_results() {}
        public function next_result() {}
        public function ping() {}
        public static function poll(?array &$read, ?array &$error, array &$reject, int $seconds, int $microseconds = 0) {}
        public function prepare(string $query) {}
        public function query(string $query, int $result_mode = \MYSQLI_STORE_RESULT) {}
        public function real_connect(?string $hostname = NULL, ?string $username = NULL, ?string $password = NULL, ?string $database = NULL, ?int $port = NULL, ?string $socket = NULL, int $flags = 0) {}
        public function real_escape_string(string $string) {}
        public function reap_async_query() {}
        public function escape_string(string $string) {}
        public function real_query(string $query) {}
        public function release_savepoint(string $name) {}
        public function rollback(int $flags = 0, ?string $name = NULL) {}
        public function savepoint(string $name) {}
        public function select_db(string $database) {}
        public function set_charset(string $charset) {}
        public function options(int $option, $value) {}
        public function set_opt(int $option, $value) {}
        public function ssl_set(?string $key, ?string $certificate, ?string $ca_certificate, ?string $ca_path, ?string $cipher_algos) {}
        public function stat() {}
        public function stmt_init() {}
        public function store_result(int $mode = 0) {}
        public function thread_safe() {}
        public function use_result() {}
        public function refresh(int $flags) {}
    }

    final class mysqli_warning {
        public  string $message;
        public  string $sqlstate;
        public  int $errno;
        private function __construct() {}
        public function next(): bool {}
    }

    class mysqli_result implements \IteratorAggregate, \Traversable {
        public  int $current_field;
        public  int $field_count;
        public  ?array $lengths;
        public  string|int $num_rows;
        public  int $type;
        public function __construct(\mysqli $mysql, int $result_mode = \MYSQLI_STORE_RESULT) {}
        public function close() {}
        public function free() {}
        public function data_seek(int $offset) {}
        public function fetch_field() {}
        public function fetch_fields() {}
        public function fetch_field_direct(int $index) {}
        public function fetch_all(int $mode = \MYSQLI_NUM) {}
        public function fetch_array(int $mode = \MYSQLI_BOTH) {}
        public function fetch_assoc() {}
        public function fetch_object(string $class = 'stdClass', array $constructor_args = array ()) {}
        public function fetch_row() {}
        public function fetch_column(int $column = 0): string|int|float|false|null {}
        public function field_seek(int $index) {}
        public function free_result() {}
        public function getIterator(): \Iterator {}
    }

    class mysqli_stmt {
        public  string|int $affected_rows;
        public  string|int $insert_id;
        public  string|int $num_rows;
        public  int $param_count;
        public  int $field_count;
        public  int $errno;
        public  string $error;
        public  array $error_list;
        public  string $sqlstate;
        public  int $id;
        public function __construct(\mysqli $mysql, ?string $query = NULL) {}
        public function attr_get(int $attribute) {}
        public function attr_set(int $attribute, int $value) {}
        public function bind_param(string $types, mixed &...$vars) {}
        public function bind_result(mixed &...$vars) {}
        public function close() {}
        public function data_seek(int $offset) {}
        public function execute(?array $params = NULL) {}
        public function fetch() {}
        public function get_warnings() {}
        public function result_metadata() {}
        public function more_results() {}
        public function next_result() {}
        public function num_rows() {}
        public function send_long_data(int $param_num, string $data) {}
        public function free_result() {}
        public function reset() {}
        public function prepare(string $query) {}
        public function store_result() {}
        public function get_result() {}
    }

    const MYSQLI_READ_DEFAULT_GROUP = 5;

    const MYSQLI_READ_DEFAULT_FILE = 4;

    const MYSQLI_OPT_CONNECT_TIMEOUT = 0;

    const MYSQLI_OPT_LOCAL_INFILE = 8;

    const MYSQLI_OPT_LOAD_DATA_LOCAL_DIR = 43;

    const MYSQLI_INIT_COMMAND = 3;

    const MYSQLI_OPT_READ_TIMEOUT = 11;

    const MYSQLI_OPT_NET_CMD_BUFFER_SIZE = 202;

    const MYSQLI_OPT_NET_READ_BUFFER_SIZE = 203;

    const MYSQLI_OPT_INT_AND_FLOAT_NATIVE = 201;

    const MYSQLI_OPT_SSL_VERIFY_SERVER_CERT = 21;

    const MYSQLI_SERVER_PUBLIC_KEY = 35;

    const MYSQLI_CLIENT_SSL = 2048;

    const MYSQLI_CLIENT_COMPRESS = 32;

    const MYSQLI_CLIENT_INTERACTIVE = 1024;

    const MYSQLI_CLIENT_IGNORE_SPACE = 256;

    const MYSQLI_CLIENT_NO_SCHEMA = 16;

    const MYSQLI_CLIENT_FOUND_ROWS = 2;

    const MYSQLI_CLIENT_SSL_VERIFY_SERVER_CERT = 1073741824;

    const MYSQLI_CLIENT_SSL_DONT_VERIFY_SERVER_CERT = 64;

    const MYSQLI_CLIENT_CAN_HANDLE_EXPIRED_PASSWORDS = 4194304;

    const MYSQLI_OPT_CAN_HANDLE_EXPIRED_PASSWORDS = 37;

    const MYSQLI_STORE_RESULT = 0;

    const MYSQLI_USE_RESULT = 1;

    const MYSQLI_ASYNC = 8;

    const MYSQLI_STORE_RESULT_COPY_DATA = 16;

    const MYSQLI_ASSOC = 1;

    const MYSQLI_NUM = 2;

    const MYSQLI_BOTH = 3;

    const MYSQLI_STMT_ATTR_UPDATE_MAX_LENGTH = 0;

    const MYSQLI_STMT_ATTR_CURSOR_TYPE = 1;

    const MYSQLI_CURSOR_TYPE_NO_CURSOR = 0;

    const MYSQLI_CURSOR_TYPE_READ_ONLY = 1;

    const MYSQLI_NOT_NULL_FLAG = 1;

    const MYSQLI_PRI_KEY_FLAG = 2;

    const MYSQLI_UNIQUE_KEY_FLAG = 4;

    const MYSQLI_MULTIPLE_KEY_FLAG = 8;

    const MYSQLI_BLOB_FLAG = 16;

    const MYSQLI_UNSIGNED_FLAG = 32;

    const MYSQLI_ZEROFILL_FLAG = 64;

    const MYSQLI_AUTO_INCREMENT_FLAG = 512;

    const MYSQLI_TIMESTAMP_FLAG = 1024;

    const MYSQLI_SET_FLAG = 2048;

    const MYSQLI_NUM_FLAG = 32768;

    const MYSQLI_PART_KEY_FLAG = 16384;

    const MYSQLI_GROUP_FLAG = 32768;

    const MYSQLI_ENUM_FLAG = 256;

    const MYSQLI_BINARY_FLAG = 128;

    const MYSQLI_NO_DEFAULT_VALUE_FLAG = 4096;

    const MYSQLI_ON_UPDATE_NOW_FLAG = 8192;

    const MYSQLI_TYPE_DECIMAL = 0;

    const MYSQLI_TYPE_TINY = 1;

    const MYSQLI_TYPE_SHORT = 2;

    const MYSQLI_TYPE_LONG = 3;

    const MYSQLI_TYPE_FLOAT = 4;

    const MYSQLI_TYPE_DOUBLE = 5;

    const MYSQLI_TYPE_NULL = 6;

    const MYSQLI_TYPE_TIMESTAMP = 7;

    const MYSQLI_TYPE_LONGLONG = 8;

    const MYSQLI_TYPE_INT24 = 9;

    const MYSQLI_TYPE_DATE = 10;

    const MYSQLI_TYPE_TIME = 11;

    const MYSQLI_TYPE_DATETIME = 12;

    const MYSQLI_TYPE_YEAR = 13;

    const MYSQLI_TYPE_NEWDATE = 14;

    const MYSQLI_TYPE_ENUM = 247;

    const MYSQLI_TYPE_SET = 248;

    const MYSQLI_TYPE_TINY_BLOB = 249;

    const MYSQLI_TYPE_MEDIUM_BLOB = 250;

    const MYSQLI_TYPE_LONG_BLOB = 251;

    const MYSQLI_TYPE_BLOB = 252;

    const MYSQLI_TYPE_VAR_STRING = 253;

    const MYSQLI_TYPE_STRING = 254;

    const MYSQLI_TYPE_CHAR = 1;

    const MYSQLI_TYPE_GEOMETRY = 255;

    const MYSQLI_TYPE_VECTOR = 242;

    const MYSQLI_TYPE_JSON = 245;

    const MYSQLI_TYPE_NEWDECIMAL = 246;

    const MYSQLI_TYPE_BIT = 16;

    const MYSQLI_SET_CHARSET_NAME = 7;

    const MYSQLI_NO_DATA = 100;

    const MYSQLI_DATA_TRUNCATED = 101;

    const MYSQLI_REPORT_INDEX = 4;

    const MYSQLI_REPORT_ERROR = 1;

    const MYSQLI_REPORT_STRICT = 2;

    const MYSQLI_REPORT_ALL = 255;

    const MYSQLI_REPORT_OFF = 0;

    const MYSQLI_DEBUG_TRACE_ENABLED = 0;

    const MYSQLI_SERVER_QUERY_NO_GOOD_INDEX_USED = 16;

    const MYSQLI_SERVER_QUERY_NO_INDEX_USED = 32;

    const MYSQLI_SERVER_QUERY_WAS_SLOW = 2048;

    const MYSQLI_SERVER_PS_OUT_PARAMS = 4096;

    const MYSQLI_REFRESH_GRANT = 1;

    const MYSQLI_REFRESH_LOG = 2;

    const MYSQLI_REFRESH_TABLES = 4;

    const MYSQLI_REFRESH_HOSTS = 8;

    const MYSQLI_REFRESH_STATUS = 16;

    const MYSQLI_REFRESH_THREADS = 32;

    const MYSQLI_REFRESH_REPLICA = 64;

    const MYSQLI_REFRESH_SLAVE = 64;

    const MYSQLI_REFRESH_MASTER = 128;

    const MYSQLI_REFRESH_BACKUP_LOG = 2097152;

    const MYSQLI_TRANS_START_WITH_CONSISTENT_SNAPSHOT = 1;

    const MYSQLI_TRANS_START_READ_WRITE = 2;

    const MYSQLI_TRANS_START_READ_ONLY = 4;

    const MYSQLI_TRANS_COR_AND_CHAIN = 1;

    const MYSQLI_TRANS_COR_AND_NO_CHAIN = 2;

    const MYSQLI_TRANS_COR_RELEASE = 4;

    const MYSQLI_TRANS_COR_NO_RELEASE = 8;

    const MYSQLI_IS_MARIADB = false;

}
