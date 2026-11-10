// Copyright 2024 Trace Machina, Inc. All rights reserved.
//
// Licensed under the Functional Source License, Version 1.1, Apache 2.0 Future License (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    See LICENSE file for details
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::HashMap;
use std::time::SystemTime;

use nativelink_error::{Code, Error};
use nativelink_macro::nativelink_test;
use nativelink_proto::build::bazel::remote::execution::v2::{
    Digest, ExecuteResponse, OutputDirectory,
};
use nativelink_proto::google::longrunning::{Operation, operation};
use nativelink_proto::google::rpc::Status;
use nativelink_util::action_messages::{
    ActionResult, ActionStage, ActionState, ActionUniqueKey, ActionUniqueQualifier, DirectoryInfo,
    ExecutionMetadata, OperationId,
};
use nativelink_util::common::DigestInfo;
use nativelink_util::digest_hasher::DigestHasherFunc;
use pretty_assertions::assert_eq;

#[nativelink_test]
async fn action_state_any_url_test() -> Result<(), Error> {
    let unique_qualifier = ActionUniqueQualifier::Cacheable(ActionUniqueKey {
        execution_scope: None,
        instance_name: "foo_instance".to_string(),
        digest_function: DigestHasherFunc::Sha256,
        digest: DigestInfo::new([1u8; 32], 5),
    });
    let action_digest = unique_qualifier.digest();
    let client_id = OperationId::default();
    let operation_id = OperationId::default();
    let action_state = ActionState {
        client_operation_id: operation_id.clone(),
        // Result is only populated if has_action_result.
        stage: ActionStage::Completed(ActionResult::default()),
        action_digest,
        last_transition_timestamp: SystemTime::now(),
    };
    let operation: Operation = action_state.as_operation(client_id);

    match &operation.result {
        Some(operation::Result::Response(any)) => assert_eq!(
            any.type_url,
            "type.googleapis.com/build.bazel.remote.execution.v2.ExecuteResponse"
        ),
        other => panic!("Expected Some(Result(Any)), got: {other:?}"),
    }

    let action_state_round_trip = ActionState::try_from_operation(operation, operation_id)?;
    assert_eq!(action_state, action_state_round_trip);

    Ok(())
}

#[nativelink_test]
async fn execute_response_status_message_is_some_on_success_test() -> Result<(), Error> {
    let execute_response: ExecuteResponse = ActionStage::Completed(ActionResult {
        output_files: vec![],
        output_folders: vec![],
        output_file_symlinks: vec![],
        output_directory_symlinks: vec![],
        exit_code: 0,
        stdout_digest: DigestInfo::new([2u8; 32], 5),
        stderr_digest: DigestInfo::new([3u8; 32], 5),
        execution_metadata: ExecutionMetadata {
            worker: "foo_worker_id".to_string(),
            queued_timestamp: SystemTime::UNIX_EPOCH,
            worker_start_timestamp: SystemTime::UNIX_EPOCH,
            worker_completed_timestamp: SystemTime::UNIX_EPOCH,
            input_fetch_start_timestamp: SystemTime::UNIX_EPOCH,
            input_fetch_completed_timestamp: SystemTime::UNIX_EPOCH,
            execution_start_timestamp: SystemTime::UNIX_EPOCH,
            execution_completed_timestamp: SystemTime::UNIX_EPOCH,
            output_upload_start_timestamp: SystemTime::UNIX_EPOCH,
            output_upload_completed_timestamp: SystemTime::UNIX_EPOCH,
        },
        server_logs: HashMap::default(),
        error: None,
        message: String::new(),
    })
    .into();

    // This was once discovered to be None, which is why this test exists.
    assert_eq!(execute_response.status, Some(Status::default()));

    Ok(())
}

// REv2 `OutputDirectory.root_directory_digest`: "The digest of the encoded
// Directory proto containing the contents of the directory's root." It must
// survive the round trip through `DirectoryInfo`, which is what the scheduler
// stores and serves.
#[nativelink_test]
async fn directory_info_keeps_root_directory_digest_test() -> Result<(), Error> {
    for root_directory_digest in [None, Some(DigestInfo::new([8u8; 32], 42))] {
        let output_directory = OutputDirectory {
            path: "out".to_string(),
            tree_digest: Some(DigestInfo::new([7u8; 32], 99).into()),
            is_topologically_sorted: false,
            root_directory_digest: root_directory_digest.map(Into::into),
        };
        let directory_info = DirectoryInfo::try_from(output_directory.clone())?;
        assert_eq!(directory_info.root_directory_digest, root_directory_digest);
        assert_eq!(OutputDirectory::from(directory_info), output_directory);
    }

    // An invalid digest is an error, as for `tree_digest`.
    let invalid = OutputDirectory {
        path: "out".to_string(),
        tree_digest: Some(DigestInfo::new([7u8; 32], 99).into()),
        is_topologically_sorted: false,
        root_directory_digest: Some(Digest {
            hash: "not hex".to_string(),
            size_bytes: 1,
        }),
    };
    assert!(DirectoryInfo::try_from(invalid).is_err());

    // A record stored before the field existed still decodes.
    let mut stored = serde_json::to_value(DirectoryInfo {
        path: "out".to_string(),
        tree_digest: DigestInfo::new([7u8; 32], 99),
        root_directory_digest: None,
    })
    .map_err(|e| Error::from_std_err(Code::Internal, &e))?;
    stored
        .as_object_mut()
        .expect("DirectoryInfo serializes to an object")
        .remove("root_directory_digest");
    let decoded: DirectoryInfo =
        serde_json::from_value(stored).map_err(|e| Error::from_std_err(Code::Internal, &e))?;
    assert_eq!(decoded.root_directory_digest, None);

    Ok(())
}
