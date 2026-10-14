<?php
// Runtime declarations exported from extension: pdo_mysql

namespace Pdo {
    class Mysql extends \PDO {
        public const ATTR_USE_BUFFERED_QUERY = 1000;
        public const ATTR_LOCAL_INFILE = 1001;
        public const ATTR_INIT_COMMAND = 1002;
        public const ATTR_COMPRESS = 1003;
        public const ATTR_DIRECT_QUERY = 20;
        public const ATTR_FOUND_ROWS = 1004;
        public const ATTR_IGNORE_SPACE = 1005;
        public const ATTR_SSL_KEY = 1006;
        public const ATTR_SSL_CERT = 1007;
        public const ATTR_SSL_CA = 1008;
        public const ATTR_SSL_CAPATH = 1009;
        public const ATTR_SSL_CIPHER = 1010;
        public const ATTR_SERVER_PUBLIC_KEY = 1011;
        public const ATTR_MULTI_STATEMENTS = 1012;
        public const ATTR_SSL_VERIFY_SERVER_CERT = 1013;
        public const ATTR_LOCAL_INFILE_DIRECTORY = 1014;
        public function getWarningCount(): int {}
    }

}
