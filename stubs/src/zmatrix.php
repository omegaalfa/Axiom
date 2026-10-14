<?php
// Runtime declarations exported from extension: zmatrix

namespace ZMatrix {
    class ZMatrix {
    }

    class ZTensor implements \Stringable {
        public function __construct($dataOrShape = null) {}
        public function __toString(): string {}
        public static function clip($input, float $min, float $max) {}
        public function sum($axis = NULL, bool $keepDims = false) {}
        public function add($other) {}
        public function sub($other) {}
        public function mul($other) {}
        public function scalarMultiply(float $scalar) {}
        public function transpose() {}
        public function abs() {}
        public function sumtotal() {}
        public function mean($axis = NULL, bool $keepDims = false) {}
        public function min() {}
        public function max() {}
        public function std() {}
        public function shape() {}
        public function ndim() {}
        public function size() {}
        public function isEmpty() {}
        public function toArray() {}
        public static function zeros(array $shape) {}
        public static function ones(array $shape) {}
        public static function full(array $shape, ?float $value = null) {}
        public static function identity(int $size) {}
        public static function random(array $shape, ?float $min = null, ?float $max = null) {}
        public static function randomUniform(array $shape, float $minimum, float $maximum, int $seed) {}
        public function matmul($other) {}
        public function im2col(int $kernelHeight, int $kernelWidth, int $strideHeight = 1, int $strideWidth = 1, int $paddingHeight = 0, int $paddingWidth = 0) {}
        public function col2im(array $inputShape, int $kernelHeight, int $kernelWidth, int $strideHeight = 1, int $strideWidth = 1, int $paddingHeight = 0, int $paddingWidth = 0) {}
        public function conv2d(\ZMatrix\ZTensor $filters, ?\ZMatrix\ZTensor $bias = NULL, int $strideHeight = 1, int $strideWidth = 1, int $paddingHeight = 0, int $paddingWidth = 0) {}
        public function conv2dBackward(\ZMatrix\ZTensor $gradOutput, \ZMatrix\ZTensor $filters, int $strideHeight = 1, int $strideWidth = 1, int $paddingHeight = 0, int $paddingWidth = 0) {}
        public function maxPool2d(int $kernelHeight, int $kernelWidth, int $strideHeight, int $strideWidth, int $paddingHeight = 0, int $paddingWidth = 0) {}
        public function maxPool2dBackward(\ZMatrix\ZTensor $gradOutput, \ZMatrix\ZTensor $indices, array $inputShape) {}
        public function globalAveragePool2d(): \ZMatrix\ZTensor {}
        public static function globalAveragePool2dBackward($gradOutput, array $inputShape) {}
        public function divide($other) {}
        public function pow(float $exponent) {}
        public function sigmoid() {}
        public function sigmoidDerivative() {}
        public function toGpu() {}
        public function toCpu() {}
        public function isOnGpu(): bool {}
        public function freeDevice() {}
        public function softmax() {}
        public function softmaxDerivative() {}
        public function relu() {}
        public function tanh() {}
        public function reluDerivative() {}
        public function tanhDerivative() {}
        public function leakyRelu(?float $alpha = null) {}
        public function leakyReluDerivative(?float $alpha = null) {}
        public function exp() {}
        public function log() {}
        public function sqrt() {}
        public function reshape(array $shape) {}
        public function permute(array $axes) {}
        public function flatten(int $startAxis = 0, int $endAxis = -1) {}
        public function broadcastTo(array $shape) {}
        public function slice($axis, $start, $end) {}
        public function column(int $col_idx) {}
        public function row(int $row_idx) {}
        public function gather(array $indices) {}
        public function argsort(?int $axis = null) {}
        public function where(int $feature_index, float $threshold) {}
        public function findIndicesWhere(int $feature_index, float $threshold) {}
        public function calculateSplitGini(int $feature_index, float $threshold, \ZMatrix\ZTensor $y) {}
        public static function arr($input_data) {}
        public function fill(float $value) {}
        public static function randn(array $shape, float $mean = 0.0, float $std_dev = 1.0) {}
        public static function arange(float $arg1, ?float $arg2 = NULL, float $arg3 = 1.0) {}
        public static function linspace(float $start, float $stop, int $num = 50, bool $endpoint = true) {}
        public static function logspace(float $start, float $stop, int $num = 50, bool $endpoint = true, float $base = 10.0) {}
        public static function eye(int $N, ?int $M = NULL, int $k = 0) {}
        public function key(array $indices) {}
        public function dot($other) {}
        public function broadcast(\ZMatrix\Ztensor $bias) {}
        public function greater($other) {}
        public static function minimum($a, $b) {}
        public static function maximum($a, $b) {}
        public function mode(?int $axis = NULL) {}
        public function scalarDivide($scalar) {}
        public function adamUpdate(\ZMatrix\ZTensor $gradient, \ZMatrix\ZTensor $firstMoment, \ZMatrix\ZTensor $secondMoment, float $learningRate, float $beta1, float $beta2, float $epsilon, float $biasCorrection1, float $biasCorrection2): \ZMatrix\ZTensor {}
        public function copy() {}
        public static function safe($input, ?bool $copy = true) {}
        public static function tile(\ZMatrix\ZTensor $tensor, int $times) {}
        public function unique() {}
        public function bincount($weights = NULL) {}
        public function argmax(?int $axis = NULL) {}
        public function argmin(?int $axis = NULL) {}
        public function sort(int $axis = 0) {}
        public function isin($values) {}
        public function cumsum(?int $axis = NULL) {}
        public static function concat(array $tensors, int $axis = 0) {}
        public function uniqueCounts(): array {}
        public static function stack(array $tensors) {}
        public function variance($axis = NULL, bool $keepDims = false, int $correction = 0) {}
        public function median() {}
        public function percentile(float $q) {}
        public function histogram(int $bins = 10) {}
        public function requiresGrad(bool $requires_grad = true): void {}
        public function isRequiresGrad(): bool {}
        public function ensureGrad(): void {}
        public function zeroGrad(): void {}
        public function getGrad(): ?\ZMatrix\ZTensor {}
        public function backward(): void {}
        public static function addAutograd(object $a, object $b): \"ZMatrix\\ZTensor" {}
        public static function subAutograd(object $a, object $b): \"ZMatrix\\ZTensor" {}
        public static function mulAutograd(object $a, object $b): \"ZMatrix\\ZTensor" {}
        public static function sumAutograd(object $tensor): \"ZMatrix\\ZTensor" {}
    }

}
