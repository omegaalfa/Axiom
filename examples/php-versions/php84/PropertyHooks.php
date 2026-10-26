<?php

namespace PhpPlayground\Php84;

class Order
{
    private string $_sku = 'DEFAULT';
    private string $_title = 'untitled';
    private float $_total = 0.0;

    public string $firstName = 'John';
    public string $lastName = 'Doe';

    public string $sku {
        get => $this->_sku;
        set => $this->_sku = strtoupper($value);
    }

    public string $title {
        get {
            return ucfirst($this->_title);
        }
    }

    public string $fullName {
        get => $this->firstName . ' ' . $this->lastName;
    }

    public private(set) float $total {
        get => $this->_total;
    }

    public function setTotal(float $total): void
    {
        $this->_total = $total;
    }
}
