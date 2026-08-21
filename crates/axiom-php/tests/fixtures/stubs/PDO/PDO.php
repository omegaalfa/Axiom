<?php

class PDO
{
    public const int ATTR_ERRMODE = 3;

    public function prepare(string $query, array $options = []): PDOStatement|false {}
}

class PDOStatement
{
    public function execute(?array $params = null): bool {}
}
