import { esc, h, selectOptions } from "./dom.js";
import { t } from "./i18n.js";

export const CREDENTIAL_KINDS = [
  ["ssh", "machine_ssh", "a_key_or_a_password_or_both"],
  ["vault", "ansible_vault", ""],
  ["scm", "source_control", "source_control_credential_hint"],
  ["net", "network", ""],
  ["aws", "amazon_web_services", ""],
  ["azure", "microsoft_azure", ""],
  ["gce", "google_compute_engine", ""],
  ["vmware", "vmware_vcenter", ""],
  ["openstack", "openstack", "empty_sets_the_domain_to_default"],
  ["registry", "container_registry", "the_job_sees_the_host_username_and_password"],
  ["github", "github_token", ""],
  ["gitlab", "gitlab_token", ""],
  ["galaxy", "ansible_galaxy", "empty_keeps_https_galaxy_ansible_com"],
  ["kubernetes", "kubernetes", ""],
  ["insights", "red_hat_insights", ""],
  ["env", "environment_variables", ""],
  ["netbox", "netbox", "netbox_credential_address_hint"],
];

export function credentialKindLabel(kind) {
  const row = CREDENTIAL_KINDS.find((item) => item[0] === kind);
  return row ? t(row[1]) : kind;
}
