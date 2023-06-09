



use std::collections::HashMap;
use std::sync::Arc;
use once_cell::sync::Lazy;
use rand::{Rng, SeedableRng, RngCore};
use rand_chacha::{rand_core, ChaCha12Rng};


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>{
    
    /*

    
        shared state sharding to decrease the time lock, we can use a shard from 
        the pool to update the mutex by locking on it inside a thread (either blocking 
        using std::sync::Mutex or none blocking using tokio::sync::Mutex) and if other 
        thread wants to use it, it can use other shard instead of waiting for the locked 
        shard to gets freed, we can use try_lock() method to check that the shard is 
        currently being locked or not also we have to update the whole shards inside 
        the pool at the end of each mutex free process which is something that will
        be taken care of by semaphores, also we've used tokio mutex to lock on the 
        mutex asyncly instead of using std mutex which is a blocking manner.
        
        
        struct Data{id: String}
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Arc<tokio::sync::Mutex<Data>>>(1024);
        let data_ = Arc::new(tokio::sync::Mutex::new(Data{id: "0".to_string()}));
        tokio::spawn(async move{
            sender.send(data_).await;
        });
        tokio::select!{
            data = receiver.recv() => {
                if let Some(d) = data{
                    // we got data
                    // ...
                }
            }
        }
    
    
    */



    pub const LAZY_STATIC_SHARED_DATA: Lazy<Db> = Lazy::new(||{
        HashMap::new()
    });
    
    type Db = HashMap<i32, String>;
    let shards = 10;

    let rand_generator = Arc::new(tokio::sync::Mutex::new(ChaCha12Rng::from_entropy()));

    let (mutex_data_sender, mut mutex_data_receiver) = tokio::sync::mpsc::channel::<Db>(shards as usize);
    let (map_shards_sender, mut map_shards_receiver) = tokio::sync::broadcast::channel::<Vec<Arc<tokio::sync::Mutex<Db>>>>(shards as usize);
    
    let send_sync_map = Arc::new(tokio::sync::Mutex::new(LAZY_STATIC_SHARED_DATA.clone())); //// no need to put in Mutex since we don't want to mutate it
    let mutex_data_sender = mutex_data_sender.clone();
    
    /*
        
        initializing the map shards so we can store all
        the mutexed db instances in there and udpate it 
        during the lock acquisition inside the app.

    */
    let mut map_shards = vec![send_sync_map; shards];
    let mut current_data_length = map_shards[0].lock().await.len();


    /*

        waiting to receive the new shards from the channel 
        to update the current shard inside the whole app
        asyncly, since we're inside an eventloop this can 
        be done at any time inside the app thus we're sure
        that we'll always use an udpated version of the shards 

    */
    tokio::select!{ //// instead of using while let ... syntax
        sent_shards = map_shards_receiver.recv() => {
            if let Ok(shards) = sent_shards{
                map_shards = shards;
                current_data_length = map_shards[0].lock().await.len();
            }
        }
    }


    /*

        after finding a free mutex we'll update it then send it to the 
        downside of the mpsc job queue channel in order to update the vector 
        of shards by selecting the largest mutex data, this must be done asyncly 
        since we don't know the time of the lock acquisition, it can happen any 
        time during the app and due to having a shard of mutex data we have to 
        update the whole shards with the latest data in a none blocking manner
        since there might be other mutex-es that are in a lock process.    
    
    */
    tokio::spawn(async move{
        let generator = rand_generator.clone(); 
        for idx in 0..map_shards.clone().len(){
            match map_shards[idx].clone().try_lock(){
                Ok(mut gaurd) => {

                    // generate random number
                    let mut rng = generator.lock().await;
                    let random = rng.to_owned().gen::<i32>();

                    // udpate the gaurd 
                    let value = format!("value is {}", idx);
                    gaurd.insert((idx as i32) * random, value);

                    // send the mutex to downside of the channel
                    mutex_data_sender.send(gaurd.to_owned()).await.unwrap();

                },
                Err(e) => {
                    // use other mutex instead
                    continue;
                } 
            }
        }
    });
    
    /* 

        in here we're waiting to receive the mutex data
        from the channel asyncly in order to update shards
        based on the largest mutex data to remove forks.  

    */
    tokio::spawn(async move{
        tokio::select!{ //// instead of using while let ... syntax
            mutex_data = mutex_data_receiver.recv() => {
                if let Some(largest_data) = mutex_data{
                    
                    // check that this is the largest data
                    if current_data_length < largest_data.len(){
                        
                        // update the whole shards with the largest_data
                        let new_shards = vec![Arc::new(tokio::sync::Mutex::new(largest_data)); shards];

                        // broadcast the new shards to the channel so all receivers can use the updated version
                        map_shards_sender.send(new_shards).unwrap();
                        
                    } else{

                        /* MEANS THAT NO MUTEX HAS BEEN MUTATED YET */
                        // ...
                    }

                } else{
                    
                    /* SOMETHING WENT WRONG IN SENDING TO CHANNEL */
                    // ...
                }    
            }
        }
    });



    Ok(())



}
