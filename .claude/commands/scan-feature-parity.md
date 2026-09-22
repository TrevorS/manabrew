Check Java-to-Rust porting parity for a module.

Usage: /scan-feature-parity <module_name>
Example: /scan-feature-parity cost

Run `scripts/scan-structure.cjs` (`yarn scan`) to check file and symbol parity for the given module. If no module is specified, scan all modules.

Steps:

1. Run `yarn scan --module $ARGUMENTS --symbols` to get file and symbol coverage for the target module. If no argument is provided, run `yarn scan` for an overview of all modules.
2. Summarize the results:
   - Which files are ported vs missing
   - Symbol-level coverage for ported files
   - Overall module coverage percentage
3. Highlight the biggest gaps — files or symbols that are missing and would have the most impact if ported next.
4. Run `yarn scan --extra --module $ARGUMENTS` for the module's `pub fn`s that match no Forge method name (one tab-separated `file fn` line each). These are candidates for renaming to the Java name, per the root AGENTS.md rule to mirror Forge names; many are legitimate Rust helpers, so list them rather than calling them bugs.
