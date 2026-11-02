# Mission: Design and plan a next-generation Rust-native web browser

You are the principal architect and technical research lead responsible for designing a new, high-performance, secure, privacy-focused web browser written primarily in Rust.

Your task is to independently research, evaluate, architect and produce a comprehensive development roadmap for the browser. You have autonomy over technical research, architectural decisions and planning. Do not blindly follow conventional browser architectures.

## 1. Vision

Create a browser that combines the strongest engineering ideas from:

- **Chromium:** Multi-process architecture, site isolation, sandboxing, networking, performance and web compatibility.
- **Firefox:** Security engineering, extension architecture, privacy, modularity and browser independence.
- **Servo:** Rust-native implementation, parallel rendering, memory safety and modular engine design.
- **Ladybird:** Independent browser engineering, clean architecture and freedom from existing browser-engine constraints.

The objective is to build something better, not simply reproduce an existing browser.

The browser should prioritize:
- Exceptional performance and responsiveness.
- Strong security and isolation.
- Privacy by default.
- Native Rust implementation wherever practical.
- Broad modern web compatibility.
- Extensibility and customization.
- Long-term independence from Google and other browser vendors.
- Minimal maintenance overhead through automation and AI.
- No telemetry.
- No internal ads, ever.

## 2. Research and technical evaluation

Study all four projects thoroughly.

Investigate their architecture, source code, documentation, engineering decisions, performance characteristics, security models, dependencies and known limitations.

Identify:
- What should be adopted.
- What should be improved.
- What should be avoided.
- What should be independently implemented.
- Which existing components should be reused rather than rewritten.

Evaluate additional relevant projects, libraries, standards and technologies where appropriate.

Do not assume that the newest or most Rust-oriented implementation is necessarily the best.

## 3. Browser architecture

Develop a complete architectural proposal covering:

- Browser process and process management.
- Rendering engine.
- HTML, CSS and JavaScript execution.
- Graphics and GPU acceleration.
- Networking, HTTP, TLS and DNS.
- Storage, cookies, caching and databases.
- Multi-process architecture and site isolation.
- Sandboxing and permission management.
- Extension framework.
- Download and file handling.
- User interface and browser interaction.
- Profiles, sessions and synchronization.
- Accessibility.
- Developer tools.
- Cross-platform support.

Evaluate whether to build on Servo, reuse selected components from other projects, or implement independent subsystems.

Design clear interfaces between components to allow individual subsystems to be replaced or upgraded without major architectural changes.

## 4. Extensions and privacy

A major objective is to provide powerful extension capabilities without depending on Google's extension policies.

Investigate:
- Full uBlock Origin compatibility, including Manifest V2.
- WebExtensions compatibility.
- Advanced network request filtering.
- Custom filtering rules.
- Tracking and fingerprinting protection.
- Third-party cookie controls.
- Per-site permissions.
- Extension isolation and security.
- Independent extension distribution.

Do not compromise extension capabilities unnecessarily.

## 5. Performance

Develop a strategy for making the browser exceptionally fast.

Investigate:
- Parallel rendering and layout.
- Efficient memory management.
- Startup time.
- Tab switching latency.
- Memory consumption per tab.
- GPU acceleration.
- Networking optimizations.
- Caching strategies.
- Background tab resource management.
- CPU and power consumption.

Define measurable performance targets and reproducible benchmarks against Chrome, Firefox, Servo-based browsers and other relevant competitors.

## 6. Security

Treat security as a foundational architectural requirement.

Plan for:
- Memory safety.
- Process isolation.
- Sandboxing.
- Site isolation.
- Secure IPC.
- Secure extension execution.
- Dependency security.
- Supply-chain integrity.
- Secure updates.
- Threat modeling.
- Vulnerability disclosure and response.
- Fuzzing and continuous security testing.

Rust should be used wherever practical, but do not assume Rust alone guarantees security.

## 7. Autonomous AI development and maintenance

Design the project around continuous AI-assisted engineering.

Plan an autonomous agent ecosystem capable of:

- Researching technical improvements.
- Implementing independent components.
- Reviewing architecture.
- Identifying performance regressions.
- Discovering vulnerabilities.
- Monitoring upstream projects.
- Updating dependencies.
- Developing security patches.
- Writing and maintaining tests.
- Reviewing pull requests.
- Maintaining documentation.

Define agent responsibilities, permissions, coordination, verification and escalation procedures.

Investigate automated CI/CD workflows that continuously monitor:
- Rust security advisories.
- Dependency vulnerabilities.
- Chromium, Firefox, Servo and Ladybird developments.
- Web standards changes.
- Performance regressions.
- Browser compatibility issues.

AI-generated security changes must undergo independent verification and appropriate human approval before production deployment.

Minimize the amount of routine human maintenance without compromising security.

## 8. Licensing and independence

Investigate the licenses of all source projects and dependencies.

Identify legal and technical restrictions on code reuse, redistribution and modification.

Design the browser to remain independently maintainable and distributable.

Minimize unnecessary vendor lock-in and external dependencies.

## 9. Development strategy

Produce a realistic phased roadmap.

Prioritize:
1. Research and architectural design.
2. Minimal working browser.
3. Rendering and networking integration.
4. Security and process isolation.
5. Extensions and privacy.
6. Performance optimization.
7. Cross-platform support.
8. Autonomous maintenance and continuous improvement.

Define dependencies, milestones, deliverables, risks and acceptance criteria.

Avoid building everything simultaneously. Prioritize early working prototypes and incremental validation.

## 10. Expected deliverables

Produce the following:

1. Comprehensive technical research report.
2. Comparative analysis of Chromium, Firefox, Servo and Ladybird.
3. Recommended overall architecture.
4. Detailed subsystem architecture.
5. Technology and dependency selection.
6. Security and threat model.
7. Performance strategy and benchmarks.
8. Extension compatibility strategy.
9. Autonomous AI engineering architecture.
10. CI/CD and security maintenance strategy.
11. Licensing and compliance assessment.
12. Phased development roadmap.
13. Risk assessment and major technical uncertainties.
14. Proposed repository structure.

## 11. Operating principles

You have autonomy to challenge assumptions and recommend better approaches.

Be technically rigorous. Distinguish proven capabilities from experimental technologies and speculative ideas.

Prefer measurable engineering evidence over opinions.

Avoid unnecessary reinvention, but do not inherit architectural limitations simply because an existing project has already implemented something.

Do not optimize prematurely.

Prioritize security, maintainability, performance and long-term architectural independence.

The final result should be a coherent, technically defensible blueprint that an autonomous AI engineering team can use to begin building the browser.