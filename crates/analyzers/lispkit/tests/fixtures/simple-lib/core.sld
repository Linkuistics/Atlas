;; A minimal LispKit library fixture for integration testing.
;; Demonstrates: define-library, export clause, begin body,
;; exported vs non-exported defines.
(define-library (simple-lib core)
  (export greet farewell)

  (begin
    ;; Exported: `greet` and `farewell` are in the export list.
    (define (greet name)
      (string-append "Hello, " name "!"))

    (define (farewell name)
      (string-append "Goodbye, " name "!"))

    ;; Not exported: `format-greeting` is a private helper.
    (define (format-greeting prefix name)
      (string-append prefix ", " name "!"))))
