<?php

namespace Probe\Contracts {
    interface Runner
    {
        public function run(): void;
    }

    interface Saver
    {
        public function save(): void;
    }
}

namespace Probe\Traits {
    trait Runnable
    {
        public function run(): void
        {
        }
    }

    trait ExecuteTrait
    {
        public function execute(): void
        {
        }
    }

    trait RunA
    {
        public function run(): void
        {
        }
    }

    trait RunB
    {
        public function run(): void
        {
        }
    }
}

namespace Probe\Base {
    abstract class AbstractService
    {
        abstract public function save(): void;
    }

    class ConcreteRunner
    {
        public function run(): void
        {
        }
    }
}

namespace Probe\Models {
    use Probe\Base\AbstractService;
    use Probe\Base\ConcreteRunner;
    use Probe\Contracts\Runner;
    use Probe\Contracts\Saver;
    use Probe\Traits\ExecuteTrait;
    use Probe\Traits\RunA;
    use Probe\Traits\RunB;
    use Probe\Traits\Runnable;

    // 1. Faltam run() e save()
    class MissingService extends AbstractService implements Runner, Saver
    {
    }

    // 2. Métodos locais
    class LocalService extends AbstractService implements Runner, Saver
    {
        public function run(): void
        {
        }

        public function save(): void
        {
        }
    }


    // 3. Trait satisfaz run()
    class TraitService extends AbstractService implements Runner, Saver
    {
        use Runnable;
        use Probe\Models\TraitService;
        use Probe\Models\InheritedService;
        use Probe\Models\AliasService;
        use Probe\Models\InsteadOfService;
        use Probe\Models\LocalBeatsAliasService;

        public function save(): void
        {
        }
    }

    // 4. Método herdado satisfaz run()
    class InheritedService extends ConcreteRunner implements Runner
    {
    }

    // 5. Alias de trait cria run()
    class AliasService implements Runner
    {
        use ExecuteTrait {
            ExecuteTrait::execute as run;
        }
    }

    // 6. insteadof escolhe RunA::run
    class InsteadOfService implements Runner
    {
        use RunA, RunB {
            RunA::run insteadof RunB;
        }
    }

    // 7. Método local deve ganhar do alias
    class LocalBeatsAliasService implements Runner
    {
        use ExecuteTrait {
            ExecuteTrait::execute as run;
        }

        public function run(): void
        {
        }
    }
}


$trait = new TraitService();
$trait->run();
$inherited = new InheritedService();
$inherited->run();
$alias = new AliasService();
$alias->run();
$instead = new InsteadOfService();
$instead->run();
$local = new LocalBeatsAliasService();
$local->run();
