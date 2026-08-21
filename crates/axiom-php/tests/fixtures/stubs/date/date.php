<?php

class DateTime
{
    public const ATOM = 'Y-m-d';
    /** @var non-empty-string */
    public string $timezone;
    public function format(string $format): string {}
}

class DateTimeImmutable {}
class DateInterval {}
