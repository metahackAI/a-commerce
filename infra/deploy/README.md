# 데모 서버 배포

데모 주소: **https://a-commerce.ailism.team**

main에 머지하면 자동으로 배포됩니다. 다시 배포하거나 되돌릴 때는 Actions 탭의 **deploy → Run workflow** 버튼을 씁니다. 서버에 들어갈 필요는 없습니다.

## 1. 흐름

```
main 머지 / Run workflow
   │
   ▼  GitHub Actions (.github/workflows/deploy.yml, 10초 남짓)
   └─ GitHub deployment 기록 (environment: demo)
   │
   ▼  데모 서버의 배포기 (infra/deploy/deployer.sh, 20초마다 확인)
   ├─ 배포 clone을 그 sha로 checkout
   ├─ docker compose build → acom/acom-{gateway,commerce,agent,web}:<sha>, 그리고 같은 이미지에 :acom-demo 태그
   │   (서버의 BuildKit 캐시가 남아 있어서 안 바뀐 서비스는 같은 이미지가 나옴. 컨테이너는 고정 태그
   │    :acom-demo 로 띄우므로 이미지 id 가 바뀐 서비스만 재시작됨)
   ├─ docker compose up -d (compose.yaml + compose.deploy.yaml)
   ├─ 모든 서비스 healthy + gateway /readyz 확인 (최대 240초)
   └─ 결과를 deployment 상태로 보고 → 저장소 오른쪽 Environments 의 demo 에 표시
```

- **서버는 밖으로 나가는 요청만 합니다**(GitHub API, 베이스 이미지 레지스트리). GitHub에 서버 접속 정보(SSH 키 등)를 두지 않고, 서버에 들어오는 포트도 열지 않습니다. public 저장소라 self-hosted runner는 쓰지 않습니다(fork PR이 서버에서 코드를 돌릴 수 있음). 배포기는 deploy workflow가 기록한 sha(main 머지, 쓰기 권한자의 Run workflow)만 빌드합니다.
- **왜 서버에서 빌드하나:** GitHub runner는 실행마다 캐시가 비고, 캐시를 받고 올리는 데만 2분이 넘게 걸렸습니다(Rust 의존성 층 1GB). 서버는 층 캐시가 계속 남아 있어서 코드 한 줄 변경이 1분 안쪽입니다.
- **무엇이 다시 빌드되나:** 매번 `compose build`를 하지만 바뀐 층만 실제로 빌드됩니다. Rust는 의존성(cargo-chef 층)과 우리 코드가 따로 캐시되어, `Cargo.toml`/`Cargo.lock`이 그대로면 우리 크레이트만 다시 컴파일합니다. 문서만 바뀌면 모든 이미지가 그대로라 컨테이너도 그대로입니다.
- **마이그레이션:** 백엔드가 바뀌면 `migrate`가 먼저 돌고 끝난 뒤 서비스가 뜹니다(compose.yaml 그대로).
- 공개 주소 경로: Cloudflare(터널) → 서버 Caddy → gateway(`172.17.0.1:18180`, 도커 브리지에만 열림).

## 2. 쓰는 법

| 하고 싶은 것 | 방법 |
|---|---|
| 배포 | main에 머지. 보통 1~3분 뒤 반영(Rust 의존성이 바뀌면 더 김) |
| 진행 상황 | Actions 탭 deploy 실행 → 서버 반영은 저장소 첫 화면 오른쪽 **Environments → demo** |
| 같은 것 다시 배포 | Actions → deploy → Run workflow (sha 비움) |
| 되돌리기 | Run workflow 에서 `sha` 에 이전 main 커밋을 넣음. 최근 5번 배포한 sha는 이미지가 남아 있어 바로 |
| 브랜치를 데모에 올려 보기 | Run workflow 에서 브랜치를 고름. 다음 main 머지 때 main으로 돌아옴 |

실패하면 Environments 의 demo 상태에 마지막 로그 한 줄이 나옵니다. 전체 로그는 서버의 `~/deploy/state/logs/<deployment id>.log`.

## 3. 서버 구성 (한 번만, 서버 담당자)

```bash
# 배포 clone (개발용 clone과 따로). .env 와 infra/secrets 는 git이 무시하므로 checkout 해도 남음
git clone https://github.com/metahackAI/a-commerce.git ~/deploy/a-commerce
cd ~/deploy/a-commerce && cp .env.example .env && infra/scripts/gen-keys.sh
# .env: ACOM_HTTP_PORT=172.17.0.1:18180, ACOM_ENV=demo, DB 비밀번호, 모델 엔드포인트와 키

# 배포기 (배포 clone 밖에 설치한 사본을 실행: clone이 checkout 되는 동안 스크립트가 바뀌지 않게)
install -D -m 755 infra/deploy/deployer.sh ~/deploy/bin/acom-deployer.sh
cat > ~/deploy/deployer.env <<'EOF'
ACOM_PUBLIC_URL=https://a-commerce.ailism.team
ACOM_READY_URL=http://172.17.0.1:18180/readyz
EOF
cp infra/deploy/acom-deployer.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now acom-deployer
journalctl --user -u acom-deployer -f
```

- `gh`가 저장소 쓰기 권한이 있는 계정으로 로그인돼 있어야 합니다(deployment 상태 보고).
- 서버의 이미지는 최근 배포한 sha 5개 것만 남기고 지웁니다(`ACOM_DEPLOY_KEEP_IMAGES`). 빌드 캐시는 `docker builder prune` 으로 따로 정리합니다(지우면 다음 배포가 처음부터 빌드).
- `deployer.sh`를 고치면 위 `install` 줄을 다시 실행하고 `systemctl --user restart acom-deployer`.
- 데이터베이스는 compose 볼륨 `acom-demo_postgres-data`. `docker compose -p acom-demo ... down -v`는 데모 데이터를 지웁니다.
