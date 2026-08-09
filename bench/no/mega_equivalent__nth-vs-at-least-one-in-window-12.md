# equivalent: exactly 13th-from-end is a  vs  some a in last 13 chars — NOT equal
equivalent '(a|b)*a(a|b){12}' '(a|b)*a(a|b){0,12}'
