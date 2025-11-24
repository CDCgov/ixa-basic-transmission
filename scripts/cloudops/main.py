from cfa.cloudops import CloudClient
from datetime import datetime

'''
This script needs the following in action secrets
AZURE_BATCH_ACCOUNT=
AZURE_USER_ASSIGNED_IDENTITY=
AZURE_SUBNET_ID=
AZURE_CLIENT_ID=
AZURE_KEYVAULT_NAME=
AZURE_KEYVAULT_SP_SECRET_ID=

# Azure Blob storage config
AZURE_BLOB_STORAGE_ACCOUNT=

# Azure container registry config
AZURE_CONTAINER_REGISTRY_ACCOUNT=

# Azure SP info
AZURE_TENANT_ID=
AZURE_SUBSCRIPTION_ID=
'''


DOCKER_IMAGE_NAME = "ixa-basic-transmission-bench"
REGISTRY_NAME = "cfaprdbatchcr"
POOL_NAME = "ixa-basic-transmission-pool1"
# add suffix with current date-time to job name to make it unique
JOB_NAME = "ixa-basic-transmission-job-" + datetime.now().strftime("%Y%m%d-%H%M%S") 

def main():
    # initialize
    cc = CloudClient(use_federated=True)

    cc.upload_files(
        files = "docker_setup.sh",
        container_name = "input-test",
        local_root_dir = "./",
        location_in_blob = "ixa-basic-transmission"
    )

    cc.create_pool(
        pool_name = POOL_NAME,
        mounts = ['input-test', ''],
        container_image_name = "rust:slim",
        vm_size = "standard_d8s_v3",
        max_autoscale_nodes = 1,
        autoscale = True
    )

    cc.create_job(
        job_name =JOB_NAME,
        pool_name = POOL_NAME,
        exist_ok = True,
        mark_complete_after_tasks_run = True
    )

    cc.add_task(
        job_name = JOB_NAME,
        command_line = "/input-test/ixa-basic-transmission/docker_setup.sh"
    )

    # cc.monitor_job(
    #     job_name = JOB_NAME,
    #     download_task_output = True,
    # )

    # get the stdout/stderr and print them?

    # delete the job
    # cc.delete_job(JOB_NAME)

    # delete the pool
    # cc.delete_pool(POOL_NAME)

if __name__ == "__main__":
    main()
