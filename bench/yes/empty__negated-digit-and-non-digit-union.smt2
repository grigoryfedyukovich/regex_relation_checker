; Auto-generated from regex-relation benchmark
; query: empty '[^\\d\\D]'
; expected folder answer: yes
(set-logic QF_S)
(set-info :smt-lib-version 2.6)
(declare-const x String)
(assert (str.in_re x (re.comp (re.union (re.range "0" "9") (re.comp (re.range "0" "9"))))))
(set-info :status unsat)
(check-sat)
