name: Bug report
description: Report a problem with Odysync
title: "[Bug] "
labels: ["type:bug"]
body:
  - type: textarea
    attributes:
      label: What happened?
      description: Describe the issue and expected behavior.
    validations:
      required: true
  - type: input
    attributes:
      label: Version
      placeholder: "1.2.0"
  - type: textarea
    attributes:
      label: Steps to reproduce
  - type: textarea
    attributes:
      label: Logs or diagnostics
      description: Attach the diagnostics zip if possible (run with --diagnostics).