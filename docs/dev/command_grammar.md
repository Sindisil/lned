cmd : [q]
    | [uU] print_sfx?
    | [eE] (filename | [!] shell_command)?
    | [f]([ ]filename)?
    | [!] shell_command
    | [!][!]
    | ln_expr? [=] print_sfx?
    | ln_expr? [aix\n]
    | ln_expr? [rw] (filename | [!] shell_command)?
    | ln_expr? [z] number? print_sfx?
    | ln_addr? [cdjJnpXy] print_sfx?
    | ln_addr? [R] (globbed_filename | [!] shell_command)?
    | ln_addr? [g][^ \n]regex[^ \n] command_list print_sfx?
    | ln_addr? [mt] ln_expr? print_sfx?
    | ln_addr? [sv][^ \n]regex[^ \n]replacement[^ \n]([g] | number)? print_sfx?
    | ln_addr? [s] (number | [g])
ln_addr : ln_expr | ln_span
ln_span : [%] | ln_expr? [,;] ln_expr?
ln_expr : ln_spec offset?
ln_spec : number | [/]regex[/] | [?]regex[?] | [.$]
offset : op+ number?
op : [+-]
number : [0-9]+
print_sfx : [np]
