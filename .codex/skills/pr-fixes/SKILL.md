---
description: "Address PR review comments, make fixes, reply, and resolve"
argument-hint: "<pr-number>"
model: claude-opus-4-5-20251101
---

# Address PR Review Comments

Automates addressing pull request review feedback: analyze comments, make fixes, reply to explain changes, resolve threads, and request re-review from @codex.

## Prerequisites

- Must have `gh` CLI authenticated
- Must be in a git repository with a GitHub remote

## Step 1: Validate Input

**If $ARGUMENTS is empty:**
- Use AskUserQuestion to ask for the PR number
- Validate it's a number

**If $ARGUMENTS is provided:**
- Extract PR number from $ARGUMENTS
- Validate it's a valid number

## Step 2: Fetch PR Review Threads

Run this GraphQL query to get all unresolved review threads:

```bash
gh api graphql -f query='
query {
  repository(owner: "{owner}", name: "{repo}") {
    pullRequest(number: PR_NUMBER) {
      id
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          path
          line
          comments(first: 10) {
            nodes {
              id
              databaseId
              body
              author {
                login
              }
            }
          }
        }
      }
    }
  }
}'
```

Filter to only unresolved threads (`isResolved: false`).

If there are no unresolved threads, inform the user and exit.

## Step 3: Analyze and Categorize Comments

For each unresolved thread:

1. Read the comment body to understand the feedback
2. Identify the file path and line number
3. Read the relevant file section to understand the context
4. Categorize the type of feedback:
    - **Code change needed** - requires file modification
    - **Documentation** - needs comment/doc update
    - **Question** - requires explanation only, no code change
    - **Disagree/Won't fix** - ASK USER before responding

Present a summary to the user showing:
- Number of comments found
- File paths affected
- Brief description of each comment

**IMPORTANT**: For any comment where you disagree or think "won't fix" is appropriate, use AskUserQuestion to get user confirmation before replying. Never auto-resolve disagreements.

## Step 4: Address Each Comment

For each comment requiring action:

### 4a. Read the relevant file

Use the Read tool to understand the context around the specified line.

### 4b. Make the fix

Use the Edit tool to make the necessary changes based on the feedback.

### 4c. Reply to the comment

Use REST API to reply to the comment explaining what was done:

```bash
gh api --method POST \
  repos/{owner}/{repo}/pulls/PR_NUMBER/comments/COMMENT_DB_ID/replies \
  -f body='Fixed: [explanation of what was changed and why]'
```

Keep replies concise but informative. Explain WHAT was changed and WHY.

### 4d. Resolve the thread

Use GraphQL mutation to resolve the thread:

```bash
gh api graphql -f query='
mutation {
  resolveReviewThread(input: {threadId: "THREAD_NODE_ID"}) {
    thread { isResolved }
  }
}'
```

## Step 5: Commit Changes

After all fixes are made:

1. Stage all changed files with `git add`
2. Create a commit with a descriptive message following conventional commits format:

```bash
git commit -m "fix: address PR review feedback

- [list each fix made]
- [reference comment IDs if helpful]

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>"
```

3. Push the changes to the remote branch

## Step 6: Request Re-Review

Add a comment to the PR requesting re-review from @codex:

```bash
gh pr comment PR_NUMBER --body '@codex review'
```

## Step 7: Summary

Present a summary to the user:

- Number of comments addressed
- Files modified
- Commit hash created
- Any comments that were NOT addressed (with reasons)
- Link to the PR

## Error Handling

- If `gh` CLI is not authenticated, instruct user to run `gh auth login`
- If PR number is invalid, show error and ask for correct number
- If a thread fails to resolve, log the error but continue with other threads
- If commit fails, show the error and suggest manual resolution