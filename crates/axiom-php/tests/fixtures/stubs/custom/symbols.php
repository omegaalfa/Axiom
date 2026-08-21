<?php

namespace Foo\Bar;

interface Contract {}
trait Shared {}
enum Status: string { case Active = 'active'; }
class Baz implements Contract { use Shared; }
