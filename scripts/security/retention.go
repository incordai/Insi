// Runtime goroutine-retention controls use a closed channel as the fixed cleanup path.
package main

import (
    "encoding/json"
    "os"
    "runtime"
    "sync"
)

// Measure blocked workers after startup, before finally releasing every fixture goroutine.
func main() {
    baseline := runtime.NumGoroutine()
    done := make(chan struct{})
    var ready, finished sync.WaitGroup
    ready.Add(64)
    finished.Add(64)
    for i := 0; i < 64; i++ {
        go func() { defer finished.Done(); ready.Done(); <-done }()
    }
    ready.Wait()
    fixed := len(os.Args) > 1 && os.Args[1] == "fixed"
    if fixed { close(done); finished.Wait() }
    retained := runtime.NumGoroutine() - baseline
    if !fixed { close(done); finished.Wait() }
    _ = json.NewEncoder(os.Stdout).Encode(map[string]int{"retained_goroutines": retained})
}
