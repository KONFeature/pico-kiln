#!/usr/bin/env python3
"""
remove_prints.py
AST-based transformer to remove all print() calls from Python code.
More robust than regex - properly handles Python syntax.
"""

import ast
import sys
from pathlib import Path


class PrintRemover(ast.NodeTransformer):
    """AST transformer that removes print() function calls."""

    def visit_Expr(self, node):
        """
        Visit expression statements and remove print calls.

        This handles standalone print statements like:
            print("hello")
        """
        # Check if this is a print call
        if isinstance(node.value, ast.Call):
            if isinstance(node.value.func, ast.Name) and node.value.func.id == 'print':
                # Return None to remove this node
                return None

        # Keep other expressions
        return self.generic_visit(node)

    def _ensure_body_not_empty(self, body):
        """Ensure a body list is not empty, add Pass if needed."""
        if not body:
            return [ast.Pass()]
        return body

    def visit_If(self, node):
        """Visit if statements and ensure bodies aren't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        # Only add pass to orelse if it exists and became empty
        if node.orelse:
            node.orelse = self._ensure_body_not_empty(node.orelse)
        return node

    def visit_While(self, node):
        """Visit while loops and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        if node.orelse:
            node.orelse = self._ensure_body_not_empty(node.orelse)
        return node

    def visit_For(self, node):
        """Visit for loops and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        if node.orelse:
            node.orelse = self._ensure_body_not_empty(node.orelse)
        return node

    def visit_FunctionDef(self, node):
        """Visit function definitions and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        return node

    def visit_AsyncFunctionDef(self, node):
        """Visit async function definitions and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        return node

    def visit_With(self, node):
        """Visit with statements and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        return node

    def visit_AsyncWith(self, node):
        """Visit async with statements and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        return node

    def visit_Try(self, node):
        """Visit try statements and ensure all bodies aren't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        # Only add pass to optional clauses if they exist
        if node.orelse:
            node.orelse = self._ensure_body_not_empty(node.orelse)
        if node.finalbody:
            node.finalbody = self._ensure_body_not_empty(node.finalbody)
        for handler in node.handlers:
            handler.body = self._ensure_body_not_empty(handler.body)
        return node

    def visit_ExceptHandler(self, node):
        """Visit except handlers and ensure body isn't empty."""
        self.generic_visit(node)
        node.body = self._ensure_body_not_empty(node.body)
        return node


def remove_prints(source_code: str) -> str:
    """
    Remove all print() calls from Python source code.

    Args:
        source_code: Python source code as string

    Returns:
        Modified source code with print calls removed
    """
    # Parse the source code into an AST
    tree = ast.parse(source_code)

    # Transform the AST to remove print calls
    transformer = PrintRemover()
    new_tree = transformer.visit(tree)

    # Fix missing locations in the AST
    ast.fix_missing_locations(new_tree)

    # Convert back to source code
    import astor
    return astor.to_source(new_tree)


def remove_prints_simple(source_code: str) -> str:
    """
    Fallback method using compile() - simpler but less pretty output.
    Works without astor dependency.
    """
    tree = ast.parse(source_code)
    transformer = PrintRemover()
    new_tree = transformer.visit(tree)
    ast.fix_missing_locations(new_tree)

    # Compile and decompile (loses formatting but works)
    code = compile(new_tree, '<string>', 'exec')

    # We need to unparse it - try ast.unparse (Python 3.9+)
    try:
        return ast.unparse(new_tree)
    except AttributeError:
        # Python < 3.9, need astor
        raise ImportError("Python < 3.9 requires 'astor' package. Install with: pip install astor")


def main():
    if len(sys.argv) != 3:
        print("Usage: remove_prints.py <input_file> <output_file>")
        sys.exit(1)

    input_path = Path(sys.argv[1])
    output_path = Path(sys.argv[2])

    if not input_path.exists():
        print(f"Error: {input_path} does not exist")
        sys.exit(1)

    # Read source code
    with open(input_path, 'r', encoding='utf-8') as f:
        source = f.read()

    # Remove prints
    try:
        modified = remove_prints_simple(source)
    except ImportError as e:
        print(f"Error: {e}")
        sys.exit(1)

    # Write output
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, 'w', encoding='utf-8') as f:
        f.write(modified)


if __name__ == "__main__":
    main()
