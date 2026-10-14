<?php
// Runtime declarations exported from extension: date

namespace {
    function strtotime(string $datetime, ?int $baseTimestamp = NULL): int|false {}

    function date(string $format, ?int $timestamp = NULL): string {}

    function idate(string $format, ?int $timestamp = NULL): int|false {}

    function gmdate(string $format, ?int $timestamp = NULL): string {}

    function mktime(int $hour, ?int $minute = NULL, ?int $second = NULL, ?int $month = NULL, ?int $day = NULL, ?int $year = NULL): int|false {}

    function gmmktime(int $hour, ?int $minute = NULL, ?int $second = NULL, ?int $month = NULL, ?int $day = NULL, ?int $year = NULL): int|false {}

    function checkdate(int $month, int $day, int $year): bool {}

    function strftime(string $format, ?int $timestamp = NULL): string|false {}

    function gmstrftime(string $format, ?int $timestamp = NULL): string|false {}

    function time(): int {}

    function localtime(?int $timestamp = NULL, bool $associative = false): array {}

    function getdate(?int $timestamp = NULL): array {}

    function date_create(string $datetime = 'now', ?\DateTimeZone $timezone = NULL): \DateTime|false {}

    function date_create_immutable(string $datetime = 'now', ?\DateTimeZone $timezone = NULL): \DateTimeImmutable|false {}

    function date_create_from_format(string $format, string $datetime, ?\DateTimeZone $timezone = NULL): \DateTime|false {}

    function date_create_immutable_from_format(string $format, string $datetime, ?\DateTimeZone $timezone = NULL): \DateTimeImmutable|false {}

    function date_parse(string $datetime): array {}

    function date_parse_from_format(string $format, string $datetime): array {}

    function date_get_last_errors(): array|false {}

    function date_format(\DateTimeInterface $object, string $format): string {}

    function date_modify(\DateTime $object, string $modifier): \DateTime|false {}

    function date_add(\DateTime $object, \DateInterval $interval): \DateTime {}

    function date_sub(\DateTime $object, \DateInterval $interval): \DateTime {}

    function date_timezone_get(\DateTimeInterface $object): \DateTimeZone|false {}

    function date_timezone_set(\DateTime $object, \DateTimeZone $timezone): \DateTime {}

    function date_offset_get(\DateTimeInterface $object): int {}

    function date_diff(\DateTimeInterface $baseObject, \DateTimeInterface $targetObject, bool $absolute = false): \DateInterval {}

    function date_time_set(\DateTime $object, int $hour, int $minute, int $second = 0, int $microsecond = 0): \DateTime {}

    function date_date_set(\DateTime $object, int $year, int $month, int $day): \DateTime {}

    function date_isodate_set(\DateTime $object, int $year, int $week, int $dayOfWeek = 1): \DateTime {}

    function date_timestamp_set(\DateTime $object, int $timestamp): \DateTime {}

    function date_timestamp_get(\DateTimeInterface $object): int {}

    function timezone_open(string $timezone): \DateTimeZone|false {}

    function timezone_name_get(\DateTimeZone $object): string {}

    function timezone_name_from_abbr(string $abbr, int $utcOffset = -1, int $isDST = -1): string|false {}

    function timezone_offset_get(\DateTimeZone $object, \DateTimeInterface $datetime): int {}

    function timezone_transitions_get(\DateTimeZone $object, int $timestampBegin = \PHP_INT_MIN, int $timestampEnd = 2147483647): array|false {}

    function timezone_location_get(\DateTimeZone $object): array|false {}

    function timezone_identifiers_list(int $timezoneGroup = \DateTimeZone::ALL, ?string $countryCode = NULL): array {}

    function timezone_abbreviations_list(): array {}

    function timezone_version_get(): string {}

    function date_interval_create_from_date_string(string $datetime): \DateInterval|false {}

    function date_interval_format(\DateInterval $object, string $format): string {}

    function date_default_timezone_set(string $timezoneId): bool {}

    function date_default_timezone_get(): string {}

    function date_sunrise(int $timestamp, int $returnFormat = \SUNFUNCS_RET_STRING, ?float $latitude = NULL, ?float $longitude = NULL, ?float $zenith = NULL, ?float $utcOffset = NULL): string|int|float|false {}

    function date_sunset(int $timestamp, int $returnFormat = \SUNFUNCS_RET_STRING, ?float $latitude = NULL, ?float $longitude = NULL, ?float $zenith = NULL, ?float $utcOffset = NULL): string|int|float|false {}

    function date_sun_info(int $timestamp, float $latitude, float $longitude): array {}

    interface DateTimeInterface {
        public const ATOM = 'Y-m-d\\TH:i:sP';
        public const COOKIE = 'l, d-M-Y H:i:s T';
        public const ISO8601 = 'Y-m-d\\TH:i:sO';
        public const ISO8601_EXPANDED = 'X-m-d\\TH:i:sP';
        public const RFC822 = 'D, d M y H:i:s O';
        public const RFC850 = 'l, d-M-y H:i:s T';
        public const RFC1036 = 'D, d M y H:i:s O';
        public const RFC1123 = 'D, d M Y H:i:s O';
        public const RFC7231 = 'D, d M Y H:i:s \\G\\M\\T';
        public const RFC2822 = 'D, d M Y H:i:s O';
        public const RFC3339 = 'Y-m-d\\TH:i:sP';
        public const RFC3339_EXTENDED = 'Y-m-d\\TH:i:s.vP';
        public const RSS = 'D, d M Y H:i:s O';
        public const W3C = 'Y-m-d\\TH:i:sP';
        public function format(string $format);
        public function getTimezone();
        public function getOffset();
        public function getTimestamp();
        public function getMicrosecond(): int;
        public function diff(\DateTimeInterface $targetObject, bool $absolute = false);
        public function __wakeup();
        public function __serialize(): array;
        public function __unserialize(array $data): void;
    }

    class DateTime implements \DateTimeInterface {
        public function __construct(string $datetime = 'now', ?\DateTimeZone $timezone = NULL) {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __wakeup() {}
        public static function __set_state(array $array) {}
        public static function createFromImmutable(\DateTimeImmutable $object) {}
        public static function createFromInterface(\DateTimeInterface $object): \DateTime {}
        public static function createFromFormat(string $format, string $datetime, ?\DateTimeZone $timezone = NULL) {}
        public static function createFromTimestamp(int|float $timestamp) {}
        public static function getLastErrors() {}
        public function format(string $format) {}
        public function modify(string $modifier) {}
        public function add(\DateInterval $interval) {}
        public function sub(\DateInterval $interval) {}
        public function getTimezone() {}
        public function setTimezone(\DateTimeZone $timezone) {}
        public function getOffset() {}
        public function getMicrosecond(): int {}
        public function setTime(int $hour, int $minute, int $second = 0, int $microsecond = 0) {}
        public function setDate(int $year, int $month, int $day) {}
        public function setISODate(int $year, int $week, int $dayOfWeek = 1) {}
        public function setTimestamp(int $timestamp) {}
        public function setMicrosecond(int $microsecond): static {}
        public function getTimestamp() {}
        public function diff(\DateTimeInterface $targetObject, bool $absolute = false) {}
    }

    class DateTimeImmutable implements \DateTimeInterface {
        public function __construct(string $datetime = 'now', ?\DateTimeZone $timezone = NULL) {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __wakeup() {}
        public static function __set_state(array $array) {}
        public static function createFromFormat(string $format, string $datetime, ?\DateTimeZone $timezone = NULL) {}
        public static function createFromTimestamp(int|float $timestamp) {}
        public static function getLastErrors() {}
        public function format(string $format) {}
        public function getTimezone() {}
        public function getOffset() {}
        public function getTimestamp() {}
        public function getMicrosecond(): int {}
        public function diff(\DateTimeInterface $targetObject, bool $absolute = false) {}
        public function modify(string $modifier) {}
        public function add(\DateInterval $interval) {}
        public function sub(\DateInterval $interval) {}
        public function setTimezone(\DateTimeZone $timezone) {}
        public function setTime(int $hour, int $minute, int $second = 0, int $microsecond = 0) {}
        public function setDate(int $year, int $month, int $day) {}
        public function setISODate(int $year, int $week, int $dayOfWeek = 1) {}
        public function setTimestamp(int $timestamp) {}
        public function setMicrosecond(int $microsecond): static {}
        public static function createFromMutable(\DateTime $object) {}
        public static function createFromInterface(\DateTimeInterface $object): \DateTimeImmutable {}
    }

    class DateTimeZone {
        public const AFRICA = 1;
        public const AMERICA = 2;
        public const ANTARCTICA = 4;
        public const ARCTIC = 8;
        public const ASIA = 16;
        public const ATLANTIC = 32;
        public const AUSTRALIA = 64;
        public const EUROPE = 128;
        public const INDIAN = 256;
        public const PACIFIC = 512;
        public const UTC = 1024;
        public const ALL = 2047;
        public const ALL_WITH_BC = 4095;
        public const PER_COUNTRY = 4096;
        public function __construct(string $timezone) {}
        public function getName() {}
        public function getOffset(\DateTimeInterface $datetime) {}
        public function getTransitions(int $timestampBegin = \PHP_INT_MIN, int $timestampEnd = 2147483647) {}
        public function getLocation() {}
        public static function listAbbreviations() {}
        public static function listIdentifiers(int $timezoneGroup = \DateTimeZone::ALL, ?string $countryCode = NULL) {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __wakeup() {}
        public static function __set_state(array $array) {}
    }

    class DateInterval {
        public function __construct(string $duration) {}
        public static function createFromDateString(string $datetime) {}
        public function format(string $format) {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __wakeup() {}
        public static function __set_state(array $array) {}
    }

    class DatePeriod implements \IteratorAggregate, \Traversable {
        public const EXCLUDE_START_DATE = 1;
        public const INCLUDE_END_DATE = 2;
        public  ?\DateTimeInterface $start;
        public  ?\DateTimeInterface $current;
        public  ?\DateTimeInterface $end;
        public  ?\DateInterval $interval;
        public  int $recurrences;
        public  bool $include_start_date;
        public  bool $include_end_date;
        public static function createFromISO8601String(string $specification, int $options = 0): static {}
        public function __construct($start, $interval = null, $end = null, $options = null) {}
        public function getStartDate() {}
        public function getEndDate() {}
        public function getDateInterval() {}
        public function getRecurrences() {}
        public function __serialize(): array {}
        public function __unserialize(array $data): void {}
        public function __wakeup() {}
        public static function __set_state(array $array) {}
        public function getIterator(): \Iterator {}
    }

    class DateError extends \Error implements \Throwable, \Stringable {
    }

    class DateObjectError extends \DateError implements \Stringable, \Throwable {
    }

    class DateRangeError extends \DateError implements \Stringable, \Throwable {
    }

    class DateException extends \Exception implements \Throwable, \Stringable {
    }

    class DateInvalidTimeZoneException extends \DateException implements \Stringable, \Throwable {
    }

    class DateInvalidOperationException extends \DateException implements \Stringable, \Throwable {
    }

    class DateMalformedStringException extends \DateException implements \Stringable, \Throwable {
    }

    class DateMalformedIntervalStringException extends \DateException implements \Stringable, \Throwable {
    }

    class DateMalformedPeriodStringException extends \DateException implements \Stringable, \Throwable {
    }

    const DATE_ATOM = 'Y-m-d\\TH:i:sP';

    const DATE_COOKIE = 'l, d-M-Y H:i:s T';

    const DATE_ISO8601 = 'Y-m-d\\TH:i:sO';

    const DATE_ISO8601_EXPANDED = 'X-m-d\\TH:i:sP';

    const DATE_RFC822 = 'D, d M y H:i:s O';

    const DATE_RFC850 = 'l, d-M-y H:i:s T';

    const DATE_RFC1036 = 'D, d M y H:i:s O';

    const DATE_RFC1123 = 'D, d M Y H:i:s O';

    const DATE_RFC7231 = 'D, d M Y H:i:s \\G\\M\\T';

    const DATE_RFC2822 = 'D, d M Y H:i:s O';

    const DATE_RFC3339 = 'Y-m-d\\TH:i:sP';

    const DATE_RFC3339_EXTENDED = 'Y-m-d\\TH:i:s.vP';

    const DATE_RSS = 'D, d M Y H:i:s O';

    const DATE_W3C = 'Y-m-d\\TH:i:sP';

    const SUNFUNCS_RET_TIMESTAMP = 0;

    const SUNFUNCS_RET_STRING = 1;

    const SUNFUNCS_RET_DOUBLE = 2;

}
