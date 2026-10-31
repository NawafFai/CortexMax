import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import type { LocalProjectCreation } from "@/shared/api/types";

type ProjectMode = "conversation" | "existing" | "new";
export type PendingProjectChannel = {
  requestId: string;
  name: string;
  description: string | null;
  channelType: "stream" | "forum";
  visibility: "open" | "private";
  root: string;
};

export function useChannelProjectDraft(active: boolean) {
  const { activeCommunity } = useCommunities();
  const identity = useIdentityQuery();
  const expectedOwner = identity.data?.pubkey;
  const expectedRelay = activeCommunity?.relayUrl;
  const [mode, setMode] = useState<ProjectMode>("conversation");
  const [root, setRoot] = useState<string | null>(null);
  const [parent, setParent] = useState<string | null>(null);
  const [folderName, setFolderName] = useState("");
  const [requestId, setRequestId] = useState<string>(() => crypto.randomUUID());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [abandonRequest, setAbandonRequest] = useState<string | null>(null);
  const generation = useRef(0);
  const scopeKey = JSON.stringify([active, expectedOwner, expectedRelay]);
  const latestScope = useRef(scopeKey);
  const resetScope = useRef(scopeKey);
  latestScope.current = scopeKey;
  useEffect(() => {
    resetScope.current = scopeKey;
    generation.current += 1;
    setMode("conversation");
    setRoot(null);
    setParent(null);
    setFolderName("");
    setRequestId(crypto.randomUUID());
    setBusy(false);
    setError(null);
    setAbandonRequest(null);
    return () => {
      generation.current += 1;
    };
  }, [scopeKey]);
  const pending = useQuery({
    queryKey: ["pending-project-channel", expectedRelay, expectedOwner],
    enabled: active && !!expectedOwner && !!expectedRelay,
    queryFn: () =>
      invokeTauri<PendingProjectChannel | null>("get_pending_project_channel", {
        expectedOwner,
        expectedRelay,
      }),
    retry: false,
    staleTime: 0,
  });
  const run = async (
    operation: () => Promise<string | null>,
    receive: (value: string) => void,
  ) => {
    if (resetScope.current !== scopeKey) return;
    const current = ++generation.current;
    setBusy(true);
    setError(null);
    try {
      const value = await operation();
      if (
        current === generation.current &&
        latestScope.current === scopeKey &&
        value !== null
      )
        receive(value);
    } catch (failure) {
      if (current === generation.current && latestScope.current === scopeKey)
        setError(String(failure));
    } finally {
      if (current === generation.current && latestScope.current === scopeKey)
        setBusy(false);
    }
  };
  const projectSelected = mode !== "conversation";
  const value: LocalProjectCreation | undefined =
    projectSelected && root && expectedOwner && expectedRelay
      ? { requestId, root, expectedOwner, expectedRelay }
      : undefined;
  return {
    mode,
    root,
    parent,
    folderName,
    setFolderName,
    busy,
    error,
    pending,
    value,
    abandonConfirmed:
      !!pending.data && abandonRequest === pending.data.requestId,
    setAbandonConfirmed: (confirmed: boolean) =>
      setAbandonRequest(confirmed ? (pending.data?.requestId ?? null) : null),
    abandon: () => {
      if (
        !pending.data ||
        pending.isFetching ||
        pending.isError ||
        abandonRequest !== pending.data.requestId
      )
        return;
      const request = pending.data.requestId;
      return run(
        async () => {
          await invokeTauri<void>("abandon_project_channel_creation", {
            expectedOwner,
            expectedRelay,
            requestId: request,
            confirmedSafe: true,
          });
          return "";
        },
        () => {
          setAbandonRequest(null);
          setRoot(null);
          setParent(null);
          setMode("conversation");
          setRequestId(crypto.randomUUID());
          void pending.refetch();
        },
      );
    },
    ready:
      !busy &&
      (!projectSelected ||
        (!!value &&
          pending.isSuccess &&
          !pending.isFetching &&
          (!pending.data || pending.data.requestId === requestId))),
    setMode: (next: ProjectMode) => {
      generation.current += 1;
      setBusy(false);
      setError(null);
      setRoot(null);
      setParent(null);
      setRequestId(crypto.randomUUID());
      setMode(next);
    },
    pick: () =>
      run(
        () => invokeTauri<string | null>("pick_channel_workspace_folder"),
        mode === "new" ? setParent : setRoot,
      ),
    createFolder: () => {
      if (!parent || !folderName.trim() || root) return;
      return run(
        () =>
          invokeTauri<string>("create_channel_project_folder", {
            parent,
            name: folderName,
          }),
        setRoot,
      );
    },
    resume: () => {
      if (!pending.data) return;
      generation.current += 1;
      setBusy(false);
      setError(null);
      setMode("existing");
      setRequestId(pending.data.requestId);
      setRoot(pending.data.root);
    },
  };
}
