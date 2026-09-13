<?php

declare(strict_types=1);

namespace Probe\TraitsA {

    trait ConflictA
    {
        public function label(): string
        {
            return 'A';
        }
    }
}

namespace Probe\TraitsB {

    trait ConflictB
    {
        public function label(): string
        {
            return 'B';
        }
    }
}

namespace Probe\Models {

    use Probe\TraitsA\ConflictA as TraitA;
    use Probe\TraitsB\ConflictB as TraitB;

    class Consumer
    {
        use TraitA, TraitB {
            TraitA::label insteadof TraitB;
            TraitB::label as labelFromB;
        }
    }

    class OverrideConsumer
    {
        use TraitA, TraitB {
            TraitA::label insteadof TraitB;
            TraitB::label as labelFromB;
        }

        public function label(): string
        {
            return 'local';
        }

        public function labelFromB(): string
        {
            return 'local-b';
        }
    }
}

namespace Probe\Usage {

    use Probe\Models\Consumer;
    use Probe\Models\OverrideConsumer;

    function exercise(): void
    {
        $consumer = new Consumer();

        $consumer->label();
        $consumer->labelFromB();

        $override = new OverrideConsumer();

        $override->label();
        $override->labelFromB();
    }
}