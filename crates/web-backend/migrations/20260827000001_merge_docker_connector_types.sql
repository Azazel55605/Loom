-- Docker host and single-container views now share one connector type. Their
-- stored configuration already distinguishes the modes: containerName is
-- present for container mode and absent for host mode.
UPDATE connector_instances
SET connector_type = 'docker'
WHERE connector_type IN ('docker-container', 'docker-host');
