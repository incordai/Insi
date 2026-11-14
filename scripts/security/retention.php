<?php
// Compare live PHP allocations after collection rather than allocator-reserved process RSS.
$before = memory_get_usage(false);
$retained = [];
$leaking = ($argv[1] ?? '') === 'leaking';
for ($i = 0; $i < 2048; $i++) {
    $block = str_repeat('x', 4096);
    if ($leaking) { $retained[] = $block; }
}
unset($block);
gc_collect_cycles();
$growth = memory_get_usage(false) - $before;
echo json_encode(['retained_bytes' => $growth]), "\n";
