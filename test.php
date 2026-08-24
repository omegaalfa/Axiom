<?php
use Fiber;
use ArrayIterator;
use AxiomTest\CustomRuntime;
use AxiomSPLTeste\ArrayIterator;


$service = new UserService();
$service->findByEmail('a@test.com');

$array = CustomRuntime::hello($name, $age);

$teste = CustomRuntime::hello($name, $age);
$teste = new CustomRuntime();

$teste = ArrayIterator::current();


    
    