terraform {
  required_version = ">= 1.5.0"
}

variable "message" {
  type        = string
  description = "Text emitted by the plan."
  default     = "koreograf"
}

output "message" {
  value       = var.message
  description = "Echo of the message variable."
}
