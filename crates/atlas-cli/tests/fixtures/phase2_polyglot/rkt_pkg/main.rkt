#lang racket/base

(provide greet)

(define (greet name)
  (string-append "hello, " name))
