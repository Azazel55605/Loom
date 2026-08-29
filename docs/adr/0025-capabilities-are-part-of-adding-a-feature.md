# 0025. A connector feature is not finished until its capabilities are declared

- Status: accepted
- Date: 2026-08-29
- Amends: [0020](./0020-connector-capability-model.md)

> **Numbering.** 0016 introduced setup guides and 0020 superseded their shape;
> this amends 0020 rather than editing it, because the useful content here is
> what a later change *broke*, which a silent edit would erase.

## Context

[ADR 0020](./0020-connector-capability-model.md) gave a setup-guide variant
toggles and `CapabilityRequirement`s, so a user can see what their proxy
configuration will and will not let Loom do. The Docker connector implemented
it correctly for everything it could do at the time.

Then three later changes added things it could do. Update management added
`applyUpdate` and the `updates` table; the resource browser added the images,
volumes and networks tables and seven actions. **None of them touched
`setup_guide()` or `capability_requirements`.** The result was a guide that:

- offered no `IMAGES`, `VOLUMES` or `NETWORKS` toggle at all, so the compose
  file it generated produced a proxy that answered `403` to three of Loom's
  five tables;
- declared no requirement for any of the eleven capabilities those changes
  introduced, so the capability summary was silent about them rather than
  wrong — which is worse, because silence reads as "fine";
- carried a test asserting `!env_vars.contains("IMAGES")` with the comment
  *"Loom never calls /images"*, which had stopped being true two features
  earlier. The test still passed. It was pinning the gap in place.

A user following the guide got a working container view and three tables that
failed with a proxy error, having been told nothing beforehand.

## Decision

**Adding an action or a resource kind to a connector that publishes a setup
guide is incomplete until that change also updates the guide's toggles and
`capability_requirements`, in the same change.** Not as a follow-up, and not as
a separate "docs" pass. It is the same obligation
[`COMPONENTS.md`](../COMPONENTS.md) carries for UI components, for the same
reason: a registry that is allowed to drift stops being worth reading.

Three tests now enforce the parts that can be enforced:

- every toggle the guide offers appears in the template it renders, so a
  switched-on toggle cannot fail to reach the generated compose file;
- every requirement names toggles that exist, and every *write* capability
  requires `post` — the empirically verified gate, below;
- every resource kind the connector declares maps to a capability the proxy
  variant declares, by an explicitly written table rather than a derived one,
  so a new kind fails the test instead of inheriting a neighbour's answer.

The third is the one that would have caught this. It cannot be fully general —
nothing can check that a *newly invented* capability key was thought about —
but it closes the case that actually occurred.

### The gate the requirements encode

LinuxServer's socket-proxy has exactly one method rule,
`http-request deny unless METH_GET || { env(POST) -m bool }`, placed after the
per-action container rules and before every category rule. `POST` is therefore
an any-method-but-`GET` gate covering `DELETE`, and there is no per-category
write toggle to offer instead. Confirmed by running the image: with
`IMAGES=VOLUMES=NETWORKS=1, POST=0` the three `GET` listings answer `200` and
every `DELETE`/`POST` on those paths answers `403`.

So: read capabilities require only their category toggle; write capabilities
require their category toggle **and** `post`.

## Consequences

The guide now offers fifteen toggles rather than twelve and declares nineteen
capabilities rather than eight, and `test_connection` live-probes the three new
listings the same way it already probed containers, logs and the host summary.
Writes stay declarative, per 0020: there is still no safe no-op delete.

The rule generalises past Docker. Any future connector-specific capability —
compose stacks, a second registry, whatever comes next — inherits the same
obligation, and the same three tests are the shape its own guard should take.

This does mean a connector with a setup guide is slightly more expensive to
extend than one without. That is the correct price: the alternative is a guide
that quietly describes an older version of the connector, which is a worse
thing to ship than no guide, because a user has no way to tell which parts of
it are still true.
