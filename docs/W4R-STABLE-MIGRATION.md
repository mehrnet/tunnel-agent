# W4r stable migration

Move the legacy W4r data lanes to a new stable cluster named `w4r`.
Keep `w4r-canary` enrolled and eligible for testing releases.

| Host | Address | Stable role | Existing data ports |
| --- | --- | --- | --- |
| ir-1 | 31.14.116.95 | Domestic | 10810 HPNSSH, 10910 WSTunnel |
| ir-4 | 62.60.222.235 | Domestic | 10810 HPNSSH, 10910 WSTunnel |
| ir-6 | 2.144.4.192 | Domestic, shared with canary | 10810 HPNSSH, 10910 WSTunnel |
| de-1 | 138.199.236.120 | Foreign, shared with canary | DE connector for all three edges |

## Instance boundary

Do not attach the existing canary node IDs to the stable cluster. The API
serves stable releases to nodes on an overlapping testing cluster, and
domestic bootstrap and renewal require one membership. Sharing hardware
requires separate enrolled node IDs and isolated runtime instances.

`TUNNEL_AGENT_INSTANCE=w4r-stable` installs to
`/opt/tunnel-agent-instances/w4r-stable` and
`/etc/tunnel-agent-instances/w4r-stable`, using
`tunnel-agent@w4r-stable.service`. systemd bind-mounts both directories over
the usual paths inside that instance only, keeping signed manifests and
their canonical paths unchanged. Stable must have its own enrollment and
TLS files. Use a bundle containing the relative release-link update.

Both instances still share the host network and firewall. Reserve distinct
management and spare listener ports across instances, including the legacy
stack. Do not enable managed public ingress in both instances on a shared
host. Leave the canary's current management ports unchanged.

## Cutover gates

1. Bootstrap the isolated stable DE connector and ir-6 instance without
   changing the running canary. Verify independent identity, management
   heartbeat, release track and rollback. Verify a testing upgrade changes
   only the canary's binary and service.
2. Enroll ir-1 and ir-4. Configure reverse HPNSSH and WSTunnel on spare
   loopback ports for all three domestic edges. Verify certificate trust,
   SOCKS egress through DE and fresh reconnects on each pair.
3. Cut over 10810 and 10910 one domestic host at a time. Keep the old
   foreign configuration available for rollback until the new processes own
   both ports and existing consumers pass live checks.
4. Remove the legacy W4r target only after all six listeners are verified.
   Recheck canary telemetry and bundle selection before and after cleanup.
