; LaTeX highlights.scm for gaviero

; Structural commands (recognized by the grammar as named nodes)
"\\begin" @keyword
"\\end" @keyword
"\\documentclass" @keyword
"\\usepackage" @keyword
"\\title" @keyword
"\\author" @keyword
"\\section" @keyword
"\\subsection" @keyword
"\\subsubsection" @keyword
"\\paragraph" @keyword
"\\chapter" @keyword
"\\part" @keyword
"\\item" @keyword
"\\label" @function
"\\ref" @function
"\\cite" @function
"\\input" @keyword
"\\include" @keyword
"\\newcommand" @keyword
"\\renewcommand" @keyword
"\\def" @keyword
"\\let" @keyword
"\\fi" @keyword

; All other commands (generic — covers \date, \maketitle, \textbf, etc.)
(command_name) @function

; Environment names
(generic_environment
  (begin
    (curly_group_text
      (text) @type)))
(generic_environment
  (end
    (curly_group_text
      (text) @type)))

; Math
(inline_formula) @number
(displayed_equation) @number
(math_environment) @number

; Packages and paths
(package_include
  (curly_group_path_list
    (path) @string))
(class_include
  (curly_group_path
    (path) @string))

; Comments
(line_comment) @comment
(block_comment) @comment

; Brackets and delimiters
["{" "}" "[" "]" "(" ")"] @punctuation.bracket
["$"] @punctuation.special

; Section titles
(section
  (curly_group
    (text) @string.special))

; Key-value options
(key_value_pair
  (text) @property)
