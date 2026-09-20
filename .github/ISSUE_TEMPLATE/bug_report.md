name: Bug report
description: Report a problem with Handover
labels: [bug]
body:
  - type: markdown
    attributes:
      value: |
        Do not include secrets. Remove pairing codes, keys, tokens, cookies,
        message contents, notification text, phone numbers, and file contents
        before submitting. See SECURITY.md.
  - type: input
    id: version
    attributes:
      label: handoverd and handoverctl version
      placeholder: Output of `handoverd --version` and `handoverctl --version`
    validations:
      required: true
  - type: dropdown
    id: backend
    attributes:
      label: Backend
      options:
        - Native
        - KDE Connect
        - Google Messages helper
        - Unknown or not applicable
    validations:
      required: true
  - type: textarea
    id: what-happened
    attributes:
      label: What happened
      placeholder: What you did, what you expected, and what happened instead
    validations:
      required: true
  - type: textarea
    id: reproduction
    attributes:
      label: Steps to reproduce
      placeholder: Numbered steps, starting from a clean daemon state if possible
  - type: textarea
    id: logs
    attributes:
      label: Logs
      description: Paste sanitized output from journalctl --user -u handoverd, with secrets removed
      render: text
