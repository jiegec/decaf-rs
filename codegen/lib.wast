    ;; Bump allocator
    ;; Current extent is store in mem[0]
    (func $_Alloc (param i32) (result i32)
        (local i32)
        (set_local 1 (i32.load (i32.const 0)))
        (i32.store (i32.const 0) (i32.add (get_local 1) (get_local 0)))
        (get_local 1)
        )

    ;; Call WASI fd_write
    (func $_PrintString (param i32) (result i32)
        (local i32 i32) ;; iov, len
        (set_local 1 (call $_Alloc (i32.const 12))) ;; iov
        (i32.store (get_local 1) (get_local 0)) ;; base
        (loop
            (if
                (i32.ne (i32.load8_u (i32.add (get_local 0) (get_local 2))) (i32.const 0))
                (then
                  (set_local 2 (i32.add (get_local 2) (i32.const 1))) ;; len ++
                  (br 1)))
        )
        (i32.store (i32.add (get_local 1) (i32.const 4)) (get_local 2)) ;; len
        (call $fd_write
              (i32.const 1) ;; stdout
              (get_local 1) ;; iov
              (i32.const 1) ;; only 1 iov
              (i32.add (get_local 1) (i32.const 8)) ;; nwritten
              )
        )

    (func $_start
        (drop (call $main)))
    (export "_start" (func $_start))
    (export "main" (func $_start))
    )
