## Philosophy

This codebase will outlive you. Every shortcut becomes someone else's burden. Patterns you establish will be copied. Corners you cut will be cut again.
Fight entropy. Leave the codebase better than you found it.

## Plan Mode

- Make the plan extremely concise. Sacrifice grammar for the sake of concision.
- At the end of each plan, give me a list of unresolved questions to answer, if any.

## Commit Messages

- Add the change type in parentheses in the commit title or description when appropriate: `(major)`, `(minor)`, or `(patch)`.
- Use `(ignore)` to exclude a commit from generated release notes.
- Never stage or commit changes without the user's explicit approval.

## Code Conventions

- Follow the existing JavaScript style and Oxlint/Oxfmt configuration.
- Keep changes focused and consistent with nearby code.
- Never edit `package.json` dependencies manually; always use the package manager CLI.

## Critical Rules

- Never build, run, or deploy the project yourself unless explicitly asked.
- Do not proactively tell the user that you did not build, stage, or commit unless they ask or it is directly relevant.
- Preserve existing behavior unless the requested change requires otherwise.
