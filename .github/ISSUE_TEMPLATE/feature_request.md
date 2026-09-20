name: Feature request
description: Suggest an improvement for Handover
labels: [enhancement]
body:
  - type: markdown
    attributes:
      value: |
        Do not include secrets. See SECURITY.md before pasting logs or examples.
  - type: input
    id: versions
    attributes:
      label: handoverd and handoverctl version
      placeholder: Output of `handoverd --version` and `handoverctl --version`
  - type: dropdown
    id: backend
    attributes:
      label: Backend
      description: Which backend does this request concern
      options:
        - Native
        - KDE Connect
        - Google Messages helper
        - Backend independent
    validations:
      required: true
  - type: textarea
    id: problem
    attributes:
      label: Problem
      placeholder: What limitation or use case motivates this request
    validations:
      required: true
  - type: textarea
    id: proposal
    attributes:
      label: Proposed change
      placeholder: What should Handover do, and how should it behave
  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives considered
      placeholder: Other approaches or workarounds, if any
