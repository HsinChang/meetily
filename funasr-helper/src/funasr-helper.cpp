// funasr-helper: persistent Fun-ASR-Nano transcription sidecar for Meetily.
//
// Derived from Fun-ASR's runtime/llama.cpp/funasr-cli/funasr-cli.cpp (Apache-2.0,
// https://github.com/QwenAudio/Fun-ASR). The signal path is upstream's, unchanged:
//
//   f32 16k mono -> kaldi fbank -> SAN-M encoder + adaptor (ggml) ->
//   low-frame-rate truncation -> [prefix | audio embeds | suffix] -> Qwen3 (llama.cpp)
//
// What differs from the upstream CLI, and why:
//
//   1. Models load ONCE at startup, then a stdio loop serves windows. Upstream loads
//      1.2 GB per invocation, which is fine for batch files and hopeless for live
//      chunked transcription. This is the whole reason the fork exists.
//   2. Audio arrives as raw f32 samples on stdin, not as a file. Meetily's pipeline
//      already produces 16 kHz mono f32, so miniaudio/funasr_audio.h is dropped.
//   3. FSMN-VAD is dropped. Meetily runs Silero VAD upstream, and benchmarking showed
//      FSMN's ~2.5 s segments actively hurt accuracy versus fixed ~15 s windows
//      (short windows starve the LLM decoder of context).
//   4. GPU offload is exposed via -ngl; upstream hardcodes n_gpu_layers = 0.
//   5. The encoder ggml backend is created once rather than per window.
//
// Protocol (line-oriented requests, JSON responses, all on stdout):
//
//   -> TRANSCRIBE <n_samples>\n  followed by n_samples*4 bytes of little-endian f32
//   <- {"type":"response","text":"..."}
//   -> PING\n      <- {"type":"pong"}
//   -> SHUTDOWN\n  <- {"type":"goodbye"}   then exit 0
//   errors:        <- {"type":"error","message":"..."}
//
// Requests are plain lines because parsing JSON in C++ would mean vendoring a JSON
// library to read three fixed message shapes. Responses are JSON so the Rust side can
// deserialize them with serde, matching the llama-helper sidecar contract.
// A {"type":"ready"} line is emitted once models are loaded.

#include "ggml.h"
#include "ggml-cpu.h"
#include "ggml-alloc.h"
#include "ggml-backend.h"
#include "gguf.h"
#include "llama.h"

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <string>
#include <vector>
#include <utility>

#if defined(_WIN32)
#include <io.h>
#include <fcntl.h>
#endif

// ======================= kaldi fbank + LFR (upstream, verbatim) =======================
static const int FS=16000, WINLEN=400, SHIFT=160, NFFT=512, NMEL=80, LFR_M=7, LFR_N=6;
static const float PREEMPH=0.97f, LOWF=20.0f, HIGHF=8000.0f;
static inline float mel(float f){ return 1127.0f*logf(1.0f+f/700.0f); }

static void fft(std::vector<float>&re,std::vector<float>&im,int n){
    for(int i=1,j=0;i<n;i++){int b=n>>1;for(;j&b;b>>=1)j^=b;j^=b;if(i<j){std::swap(re[i],re[j]);std::swap(im[i],im[j]);}}
    for(int len=2;len<=n;len<<=1){double a=-2.0*M_PI/len;float wr=cosf(a),wi=sinf(a);
        for(int i=0;i<n;i+=len){float cr=1,ci=0;for(int k=0;k<len/2;k++){
            float ur=re[i+k],ui=im[i+k];float vr=re[i+k+len/2]*cr-im[i+k+len/2]*ci,vi=re[i+k+len/2]*ci+im[i+k+len/2]*cr;
            re[i+k]=ur+vr;im[i+k]=ui+vi;re[i+k+len/2]=ur-vr;im[i+k+len/2]=ui-vi;
            float n2=cr*wr-ci*wi;ci=cr*wi+ci*wr;cr=n2;}}}
}

// returns [T x 560] row-major, sets T_out
static std::vector<float> compute_fbank(std::vector<float> wav, int & T_out) {
    for (auto & v : wav) v *= 32768.0f;
    std::vector<float> win(WINLEN);
    for (int i=0;i<WINLEN;i++) win[i]=0.54f-0.46f*cosf(2.0f*M_PI*i/(WINLEN-1));
    const int NBIN=NFFT/2+1; float bw=(float)FS/NFFT, ml=mel(LOWF), mh=mel(HIGHF), dm=(mh-ml)/(NMEL+1);
    std::vector<std::vector<float>> fb(NMEL, std::vector<float>(NBIN,0.0f));
    for(int m=0;m<NMEL;m++){float L=ml+m*dm,C=ml+(m+1)*dm,R=ml+(m+2)*dm;
        for(int k=0;k<NBIN;k++){float mf=mel(bw*k); if(mf>L&&mf<R) fb[m][k]=mf<=C?(mf-L)/(C-L):(R-mf)/(R-C);}}
    int N=wav.size(); int T=(N-WINLEN)/SHIFT+1;
    std::vector<std::vector<float>> feat(T, std::vector<float>(NMEL));
    std::vector<float> re(NFFT),im(NFFT),fr(WINLEN);
    const float fl=1.1920929e-07f;
    for(int t=0;t<T;t++){const float*s=wav.data()+t*SHIFT;
        double mn=0;for(int i=0;i<WINLEN;i++)mn+=s[i];mn/=WINLEN;
        for(int i=0;i<WINLEN;i++)fr[i]=s[i]-(float)mn;
        for(int i=WINLEN-1;i>0;i--)fr[i]-=PREEMPH*fr[i-1];fr[0]-=PREEMPH*fr[0];
        for(int i=0;i<NFFT;i++){re[i]=i<WINLEN?fr[i]*win[i]:0.0f;im[i]=0.0f;}
        fft(re,im,NFFT);
        for(int m=0;m<NMEL;m++){float e=0;for(int k=0;k<NBIN;k++)if(fb[m][k]>0)e+=fb[m][k]*(re[k]*re[k]+im[k]*im[k]);
            feat[t][m]=logf(e>fl?e:fl);}}
    // LFR (low frame rate stacking)
    const int pad=(LFR_M-1)/2; int T_lfr=(T+LFR_N-1)/LFR_N;
    std::vector<std::vector<float>> pd; pd.reserve(T+pad+LFR_M);
    for(int i=0;i<pad;i++)pd.push_back(feat[0]);
    for(int t=0;t<T;t++)pd.push_back(feat[t]);
    while((int)pd.size()<(T_lfr-1)*LFR_N+LFR_M)pd.push_back(feat[T-1]);
    int D=LFR_M*NMEL; std::vector<float> out((size_t)T_lfr*D);
    for(int i=0;i<T_lfr;i++)for(int j=0;j<LFR_M;j++)
        memcpy(&out[(size_t)i*D+j*NMEL],pd[i*LFR_N+j].data(),NMEL*sizeof(float));
    T_out=T_lfr; return out;
}

// ======================= ggml SAN-M encoder + adaptor (upstream, verbatim) =======================
struct cfg { int d_model=512,n_head=4,num_blocks=50,tp_blocks=20,kernel=11,adp_llm=1024,adp_layers=2,adp_head=8; };
struct enc_model {
    cfg c; ggml_context*ctx_w=nullptr; std::map<std::string,ggml_tensor*> t;
    ggml_tensor* g(const std::string&n){
        auto it=t.find(n);
        if(it==t.end()){fprintf(stderr,"missing tensor %s\n",n.c_str());exit(1);}
        return it->second;
    }
};
static const float LN_EPS=1e-5f;

static bool load_enc(const char*p, enc_model&m){
    gguf_init_params gp={false,&m.ctx_w}; gguf_context*g=gguf_init_from_file(p,gp); if(!g)return false;
    auto rd=[&](const char*k,int d){int i=gguf_find_key(g,k);return i<0?d:(int)gguf_get_val_u32(g,i);};
    m.c.d_model=rd("funasr.enc.output_size",512); m.c.n_head=rd("funasr.enc.attention_heads",4);
    m.c.num_blocks=rd("funasr.enc.num_blocks",50); m.c.tp_blocks=rd("funasr.enc.tp_blocks",20);
    m.c.kernel=rd("funasr.enc.kernel_size",11); m.c.adp_llm=rd("funasr.adp.llm_dim",1024);
    m.c.adp_layers=rd("funasr.adp.n_layer",2); m.c.adp_head=rd("funasr.adp.attention_heads",8);
    int n=gguf_get_n_tensors(g);
    for(int i=0;i<n;i++){const char*nm=gguf_get_tensor_name(g,i);m.t[nm]=ggml_get_tensor(m.ctx_w,nm);}
    gguf_free(g); return true;
}

static ggml_tensor* lin(ggml_context*c,ggml_tensor*w,ggml_tensor*b,ggml_tensor*x){auto y=ggml_mul_mat(c,w,x);return b?ggml_add(c,y,b):y;}
static ggml_tensor* lnorm(ggml_context*c,ggml_tensor*x,ggml_tensor*g,ggml_tensor*b){return ggml_add(c,ggml_mul(c,ggml_norm(c,x,LN_EPS),g),b);}

static ggml_tensor* sanm_attn(ggml_context*c,enc_model&m,const std::string&p,ggml_tensor*x,int T){
    const int D=m.c.d_model,H=m.c.n_head,dk=D/H,K=m.c.kernel;
    ggml_tensor*qkv=lin(c,m.g(p+"linear_q_k_v.weight"),m.g(p+"linear_q_k_v.bias"),x); size_t nb1=qkv->nb[1];
    ggml_tensor*q=ggml_cont(c,ggml_view_2d(c,qkv,D,T,nb1,0));
    ggml_tensor*k=ggml_cont(c,ggml_view_2d(c,qkv,D,T,nb1,(size_t)D*sizeof(float)));
    ggml_tensor*v=ggml_cont(c,ggml_view_2d(c,qkv,D,T,nb1,(size_t)2*D*sizeof(float)));
    const int pad=(K-1)/2; ggml_tensor*fk=m.g(p+"fsmn_block.weight");
    ggml_tensor*vp=ggml_pad_ext(c,v,0,0,pad,pad,0,0,0,0); ggml_tensor*fsmn=v;
    for(int j=0;j<K;j++){auto sl=ggml_view_2d(c,vp,D,T,vp->nb[1],(size_t)j*vp->nb[1]);
        auto wj=ggml_view_1d(c,fk,D,(size_t)j*fk->nb[1]); fsmn=ggml_add(c,fsmn,ggml_mul(c,ggml_cont(c,sl),wj));}
    q=ggml_permute(c,ggml_reshape_3d(c,q,dk,H,T),0,2,1,3); k=ggml_permute(c,ggml_reshape_3d(c,k,dk,H,T),0,2,1,3);
    ggml_tensor*vh=ggml_cont(c,ggml_permute(c,ggml_reshape_3d(c,v,dk,H,T),1,2,0,3));
    ggml_tensor*kq=ggml_soft_max(c,ggml_scale(c,ggml_mul_mat(c,k,q),1.0f/sqrtf((float)dk)));
    ggml_tensor*o=ggml_cont_2d(c,ggml_permute(c,ggml_mul_mat(c,vh,kq),0,2,1,3),D,T);
    return ggml_add(c,lin(c,m.g(p+"linear_out.weight"),m.g(p+"linear_out.bias"),o),fsmn);
}

static ggml_tensor* sanm_layer(ggml_context*c,enc_model&m,const std::string&p,ggml_tensor*x,int T,bool res){
    auto r=x; auto h=lnorm(c,x,m.g(p+"norm1.weight"),m.g(p+"norm1.bias"));
    auto sa=sanm_attn(c,m,p+"self_attn.",h,T); x=res?ggml_add(c,r,sa):sa; r=x;
    h=lnorm(c,x,m.g(p+"norm2.weight"),m.g(p+"norm2.bias"));
    h=lin(c,m.g(p+"feed_forward.w_1.weight"),m.g(p+"feed_forward.w_1.bias"),h); h=ggml_relu(c,h);
    h=lin(c,m.g(p+"feed_forward.w_2.weight"),m.g(p+"feed_forward.w_2.bias"),h); return ggml_add(c,r,h);
}

static ggml_tensor* adp_layer(ggml_context*c,enc_model&m,const std::string&p,ggml_tensor*x,int T){
    const int D=m.c.adp_llm,H=m.c.adp_head,dk=D/H; auto r=x;
    auto h=lnorm(c,x,m.g(p+"norm1.weight"),m.g(p+"norm1.bias"));
    auto q=ggml_permute(c,ggml_reshape_3d(c,lin(c,m.g(p+"self_attn.linear_q.weight"),m.g(p+"self_attn.linear_q.bias"),h),dk,H,T),0,2,1,3);
    auto k=ggml_permute(c,ggml_reshape_3d(c,lin(c,m.g(p+"self_attn.linear_k.weight"),m.g(p+"self_attn.linear_k.bias"),h),dk,H,T),0,2,1,3);
    auto vh=ggml_cont(c,ggml_permute(c,ggml_reshape_3d(c,lin(c,m.g(p+"self_attn.linear_v.weight"),m.g(p+"self_attn.linear_v.bias"),h),dk,H,T),1,2,0,3));
    auto kq=ggml_soft_max(c,ggml_scale(c,ggml_mul_mat(c,k,q),1.0f/sqrtf((float)dk)));
    auto o=ggml_cont_2d(c,ggml_permute(c,ggml_mul_mat(c,vh,kq),0,2,1,3),D,T);
    x=ggml_add(c,r,lin(c,m.g(p+"self_attn.linear_out.weight"),m.g(p+"self_attn.linear_out.bias"),o)); r=x;
    h=lnorm(c,x,m.g(p+"norm2.weight"),m.g(p+"norm2.bias"));
    h=lin(c,m.g(p+"feed_forward.w_1.weight"),m.g(p+"feed_forward.w_1.bias"),h); h=ggml_relu(c,h);
    h=lin(c,m.g(p+"feed_forward.w_2.weight"),m.g(p+"feed_forward.w_2.bias"),h); return ggml_add(c,r,h);
}

static void add_posenc(std::vector<float>&x,int T,int depth){
    double inc=log(10000.0)/(depth/2.0-1.0);
    for(int t=0;t<T;t++){double pos=t+1;for(int i=0;i<depth/2;i++){double its=exp(i*-inc),st=pos*its;
        x[(size_t)t*depth+i]+=(float)sin(st);x[(size_t)t*depth+depth/2+i]+=(float)cos(st);}}
}

// fbank [T x F] -> adaptor out [T x adp_llm] row-major.
// The ggml graph is rebuilt per window (its shape depends on T), but the backend is
// created once by the caller instead of per call as upstream does.
static std::vector<float> run_encoder(enc_model&m, ggml_backend_t be, int n_threads,
                                      std::vector<float> fbank, int T, int F, int&Dout){
    float sc=sqrtf((float)m.c.d_model); for(auto&v:fbank)v*=sc; add_posenc(fbank,T,F);
    ggml_init_params cp={(size_t)1024*1024*1024,nullptr,true}; ggml_context*c=ggml_init(cp);
    ggml_tensor*inp=ggml_new_tensor_2d(c,GGML_TYPE_F32,F,T); ggml_set_input(inp);
    ggml_tensor*x=sanm_layer(c,m,"audio_encoder.encoders0.0.",inp,T,false);
    for(int i=0;i<m.c.num_blocks-1;i++) x=sanm_layer(c,m,"audio_encoder.encoders."+std::to_string(i)+".",x,T,true);
    x=lnorm(c,x,m.g("audio_encoder.after_norm.weight"),m.g("audio_encoder.after_norm.bias"));
    for(int i=0;i<m.c.tp_blocks;i++) x=sanm_layer(c,m,"audio_encoder.tp_encoders."+std::to_string(i)+".",x,T,true);
    x=lnorm(c,x,m.g("audio_encoder.tp_norm.weight"),m.g("audio_encoder.tp_norm.bias"));
    x=lin(c,m.g("audio_adaptor.linear1.weight"),m.g("audio_adaptor.linear1.bias"),x); x=ggml_relu(c,x);
    x=lin(c,m.g("audio_adaptor.linear2.weight"),m.g("audio_adaptor.linear2.bias"),x);
    for(int i=0;i<m.c.adp_layers;i++) x=adp_layer(c,m,"audio_adaptor.blocks."+std::to_string(i)+".",x,T);
    ggml_set_output(x);
    ggml_cgraph*gf=ggml_new_graph_custom(c,32768,false); ggml_build_forward_expand(gf,x);
    ggml_gallocr_t ga=ggml_gallocr_new(ggml_backend_cpu_buffer_type()); ggml_gallocr_alloc_graph(ga,gf);
    ggml_backend_tensor_set(inp,fbank.data(),0,ggml_nbytes(inp));
    ggml_backend_cpu_set_n_threads(be,n_threads); ggml_backend_graph_compute(be,gf);
    Dout=(int)x->ne[0]; std::vector<float> out((size_t)Dout*T); ggml_backend_tensor_get(x,out.data(),0,ggml_nbytes(x));
    ggml_gallocr_free(ga); ggml_free(c); return out;
}

// ======================= LLM (llama.cpp) =======================
static int decode_batch(llama_context*ctx,int n,llama_token*tok,float*embd,int n_embd,int&n_past,bool last_logits){
    std::vector<llama_pos> pos(n); std::vector<int32_t> nsid(n,1);
    std::vector<llama_seq_id> s0(1,0); std::vector<llama_seq_id*> sid(n); std::vector<int8_t> lg(n,0);
    for(int i=0;i<n;i++){pos[i]=n_past+i;sid[i]=s0.data();}
    if(last_logits) lg[n-1]=1;
    llama_batch b={n,tok,embd,pos.data(),nsid.data(),sid.data(),lg.data()};
    int r=llama_decode(ctx,b); n_past+=n; return r;
}

// ======================= protocol helpers =======================
static std::string json_escape(const std::string & s) {
    std::string o; o.reserve(s.size()+16);
    for (unsigned char ch : s) {
        switch (ch) {
            case '"':  o += "\\\""; break;
            case '\\': o += "\\\\"; break;
            case '\n': o += "\\n";  break;
            case '\r': o += "\\r";  break;
            case '\t': o += "\\t";  break;
            case '\b': o += "\\b";  break;
            case '\f': o += "\\f";  break;
            default:
                // UTF-8 continuation/lead bytes pass through untouched; only C0 controls
                // need escaping, and they cannot appear inside a valid UTF-8 sequence.
                if (ch < 0x20) { char b[8]; snprintf(b,sizeof(b),"\\u%04x",ch); o += b; }
                else o += (char)ch;
        }
    }
    return o;
}

static void emit(const std::string & json_line) {
    fputs(json_line.c_str(), stdout);
    fputc('\n', stdout);
    fflush(stdout);
}
static void emit_error(const std::string & msg) {
    emit("{\"type\":\"error\",\"message\":\"" + json_escape(msg) + "\"}");
}

// Read exactly n bytes from stdin; false on EOF/short read.
static bool read_exact(void * dst, size_t n) {
    size_t got = 0; char * p = (char *)dst;
    while (got < n) {
        size_t r = fread(p + got, 1, n - got, stdin);
        if (r == 0) return false;
        got += r;
    }
    return true;
}

int main(int argc, char ** argv) {
    std::string enc_path, llm_path;
    std::string prefix = "<|im_start|>system\nYou are a helpful assistant.<|im_end|>\n<|im_start|>user\n语音转写：";
    int npred = 512, n_gpu_layers = 0, n_threads = 8, n_ctx = 2048;
    float rep = 1.0f;

    for (int i = 1; i < argc; i++) {
        if      (!strcmp(argv[i],"--enc")    && i+1<argc) enc_path = argv[++i];
        else if (!strcmp(argv[i],"-m")       && i+1<argc) llm_path = argv[++i];
        else if (!strcmp(argv[i],"-n")       && i+1<argc) npred = atoi(argv[++i]);
        else if (!strcmp(argv[i],"-ngl")     && i+1<argc) n_gpu_layers = atoi(argv[++i]);
        else if (!strcmp(argv[i],"-t")       && i+1<argc) n_threads = atoi(argv[++i]);
        else if (!strcmp(argv[i],"-c")       && i+1<argc) n_ctx = atoi(argv[++i]);
        else if (!strcmp(argv[i],"--rep")    && i+1<argc) rep = atof(argv[++i]);
        else if (!strcmp(argv[i],"--prompt") && i+1<argc) prefix = argv[++i];
        else {
            fprintf(stderr,"usage: %s --enc enc.gguf -m llm.gguf [-ngl N] [-t N] [-c N] [-n N] [--rep F] [--prompt S]\n",argv[0]);
            return 1;
        }
    }
    if (enc_path.empty() || llm_path.empty()) { fprintf(stderr,"missing --enc / -m\n"); return 1; }

#if defined(_WIN32)
    // stdin carries raw f32 PCM; without this Windows mangles 0x1A and CRLF.
    _setmode(_fileno(stdin),  _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
#endif

    // ---- load once ----
    enc_model em;
    if (!load_enc(enc_path.c_str(), em)) { emit_error("failed to load encoder gguf: " + enc_path); return 1; }

    ggml_backend_load_all();
    ggml_backend_t be = ggml_backend_cpu_init();
    if (!be) { emit_error("failed to init ggml cpu backend"); return 1; }

    llama_model_params mp = llama_model_default_params();
    mp.n_gpu_layers = n_gpu_layers;
    llama_model * model = llama_model_load_from_file(llm_path.c_str(), mp);
    if (!model) { emit_error("failed to load llm gguf: " + llm_path); return 1; }

    const llama_vocab * vocab = llama_model_get_vocab(model);
    llama_context_params cp = llama_context_default_params();
    cp.n_ctx = n_ctx; cp.n_batch = n_ctx; cp.n_ubatch = n_ctx;
    llama_context * ctx = llama_init_from_model(model, cp);
    if (!ctx) { emit_error("failed to create llama context"); llama_model_free(model); return 1; }

    auto sp = llama_sampler_chain_default_params();
    llama_sampler * smpl = llama_sampler_chain_init(sp);
    if (rep != 1.0f) llama_sampler_chain_add(smpl, llama_sampler_init_penalties(256, rep, 0.0f, 0.0f));
    llama_sampler_chain_add(smpl, llama_sampler_init_greedy());

    const char * suffix = "<|im_end|>\n<|im_start|>assistant\n";
    auto tokenize = [&](const char * s) {
        int n = -llama_tokenize(vocab, s, strlen(s), nullptr, 0, false, true);
        std::vector<llama_token> v(n);
        llama_tokenize(vocab, s, strlen(s), v.data(), n, false, true);
        return v;
    };
    auto pre = tokenize(prefix.c_str());
    auto suf = tokenize(suffix);

    emit("{\"type\":\"ready\"}");

    // ---- serve ----
    char line[256];
    while (fgets(line, sizeof(line), stdin)) {
        if (!strncmp(line, "PING", 4))     { emit("{\"type\":\"pong\"}"); continue; }
        if (!strncmp(line, "SHUTDOWN", 8)) { emit("{\"type\":\"goodbye\"}"); break; }

        long n_samples = 0;
        if (sscanf(line, "TRANSCRIBE %ld", &n_samples) != 1) {
            emit_error("malformed request");
            continue;
        }
        if (n_samples <= 0 || n_samples > 16000L * 60 * 30) {   // 30 min ceiling
            emit_error("invalid n_samples");
            continue;
        }

        std::vector<float> wav((size_t)n_samples);
        if (!read_exact(wav.data(), (size_t)n_samples * sizeof(float))) {
            emit_error("short read on audio payload");
            break;   // stream desynchronised — the parent must respawn us
        }

        if (n_samples < WINLEN) { emit("{\"type\":\"response\",\"text\":\"\"}"); continue; }

        int T = 0; auto fbank = compute_fbank(wav, T);
        if (T <= 0) { emit("{\"type\":\"response\",\"text\":\"\"}"); continue; }

        int D = 0; auto adp = run_encoder(em, be, n_threads, fbank, T, 560, D);

        // upstream's audio-embedding length derivation (two conv subsamplings, then /2)
        int ol = 1 + (T - 3 + 2) / 2; ol = 1 + (ol - 3 + 2) / 2;
        int n_aud = (ol - 1) / 2 + 1;

        llama_memory_clear(llama_get_memory(ctx), true);   // fresh context per window
        int n_past = 0;
        decode_batch(ctx, pre.size(), pre.data(), nullptr, 0, n_past, false);
        decode_batch(ctx, n_aud, nullptr, adp.data(), D, n_past, false);
        decode_batch(ctx, suf.size(), suf.data(), nullptr, 0, n_past, true);

        std::string text;
        llama_token tk = llama_sampler_sample(smpl, ctx, -1);
        for (int i = 0; i < npred; i++) {
            if (llama_vocab_is_eog(vocab, tk)) break;
            char buf[256];
            int k = llama_token_to_piece(vocab, tk, buf, sizeof(buf), 0, true);
            if (k > 0) text.append(buf, k);
            decode_batch(ctx, 1, &tk, nullptr, 0, n_past, true);
            tk = llama_sampler_sample(smpl, ctx, -1);
        }

        emit("{\"type\":\"response\",\"text\":\"" + json_escape(text) + "\"}");
    }

    llama_sampler_free(smpl);
    llama_free(ctx);
    llama_model_free(model);
    ggml_backend_free(be);
    if (em.ctx_w) ggml_free(em.ctx_w);
    return 0;
}
