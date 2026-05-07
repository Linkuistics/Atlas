;; LispKit library fixture for the PR-14 polyglot acceptance test.
(define-library (lk-pkg main)
  (export greet)
  (begin
    (define (greet name)
      (string-append "Hello, " name "!"))))
