**Helve** is a Kubernetes-native orchestration layer designed to deploy, manage, and scale a distributed constellation of interactive development environments (IDEs) 
and Large Language Model (LLM) engines. 

Unlike single-container setups, Helve treats your cluster as a single, unified resource, where compute, storage, and intelligence are distributed across 
multiple nodes via **K8s/K3s**.

---

## Architecture

Helve decomposes the development workflow into three distinct functional layers within your cluster:

### 1. The Intelligence Layer (AI models)
The heavy-lifting engines that power your models. Deployed as high-availability `StatefulSets` or `Deployments` with GPU-passthrough capabilities.
* **Ollama / vLLM / SGLang:** Distributed LLM inference engines.
* **Model Storage:** Persistent Volumes (PV) for weights and quantized models.

### 2. The Interface Layer (IDEs)
The interactive environments where users interact with code and data.
* **Code-Server (VS Code):** Browser-based IDE for full-stack development.
* **JupyterLab / RStudio:** Managed notebooks for data science and statistical computing.
* **Open WebUI:** The primary gateway for interacting with your cluster's LLMs.

### 3. The Gateway Layer (The Ingress)
The entry point for all cluster traffic.
* **Ingress Controller (Nginx/Traefik):** Man*n*ages SSL termination and routing.
* **Auth Proxy:** A centralized security layer to ensure only authorized researchers can access the cluster.

---

## Deployment

Helve is deployed using **Helm** or standard **Kubernetes Manifests**. It is optimized for **K3s** (for edge/resource-constrained environments) and **Standard K8s** 
(for heavy-duty GPU clusters).

### Prerequisites
* A running **Kubernetes** or **K3s** cluster.
* **Helm** installed.
* (Optional) **Intel/AMD/NVIDIA Device Plugin** (if deploying GPU-accelerated workloads).
* A **StorageClass** (e.g., Longhorn, NFS, or local-path) for persistent data.


## 🛠️ Cluster Capabilities

| Feature | Implementation | Capability |
| :--- | :--- | :--- |
| **Orchestration** | K8s / K3s | Auto-healing, auto-scaling, and rolling updates. |
| **Scalability** | Horizontal Pod Autoscaler | Scale IDE instances based on cluster load. |
| **Storage** | Persistent Volumes | Shared model weights and user-specific workspaces. |
| **Compute** | GPU Passthrough | Direct access to NVIDIA/AMD hardware for inference. |
| **Networking** | Ingress-based Routing | Single entry point for multiple specialized services. |

---

## 🌌 Roadmap: 

- [ ] **Multi-Tenancy:** Isolated namespaces for different research teams.
- [ ] **Auto-Scaling Inference:** Scale vLLM replicas based on request latency.
- [ ] **Cluster-wide GitOps:** Integration with **ArgoCD** for automated constellation updates.
- [ ] **Edge Integration:** Seamless syncing between K3s edge nodes and cloud-based K8s cores.
- [ ] **User Resource Pools** Cloud like ability to give users access to pools of resources for them to create and run models and tools

---

## 📜 License

Distributed under the MIT License. See `LICENSE` for more information.

**"In the void of raw data, Helve brings the light of intelligence."**
